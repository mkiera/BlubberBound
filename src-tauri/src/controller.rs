use crate::{engine, settings};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
use tempfile::{NamedTempFile, TempDir};

#[derive(Debug, PartialEq)]
struct Identity {
    length: u64,
    modified: std::time::SystemTime,
    file: same_file::Handle,
}

fn identity(path: &Path) -> Result<Identity, String> {
    let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if meta.file_type().is_symlink() || !meta.is_file() {
        return Err("The previous output is not a regular file. Choose another copy.".into());
    }
    Ok(Identity {
        length: meta.len(),
        modified: meta.modified().map_err(|e| e.to_string())?,
        file: same_file::Handle::from_file(fs::File::open(path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?,
    })
}

pub fn replace_file(source: &Path, target: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{
            MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
        };
        let from: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
        let to: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
        if unsafe {
            MoveFileExW(
                from.as_ptr(),
                to.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error().to_string());
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        fs::rename(source, target).map_err(|e| e.to_string())
    }
}

struct Inner {
    settings: Value,
    stored: Value,
    jobs: Vec<Value>,
    running: bool,
    closed: bool,
    stop: bool,
    installing: bool,
    error: String,
    preview: Value,
    preview_temp: Option<TempDir>,
    replacements: HashMap<String, (PathBuf, Identity)>,
}

pub struct Controller {
    inner: Mutex<Inner>,
    path: PathBuf,
    cancel: AtomicBool,
    preview_cancel: AtomicBool,
}

fn defaults() -> Value {
    let mut value = settings::defaults();
    value["output_dir"] = json!("");
    value
}
fn terminal(job: &Value) -> bool {
    matches!(
        job["status"].as_str(),
        Some("completed" | "failed" | "cancelled")
    )
}
fn text(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap_or_default().to_string()
}
fn new_job(source: &Path) -> Value {
    json!({"id":uuid::Uuid::new_v4().simple().to_string(),"source":source.to_string_lossy(),"name":source.file_name().unwrap_or_default().to_string_lossy(),"kind":"","original_size":0,"status":"probing","percent":0,"stage":"Reading file","error":"","output":"","output_size":0,"duration":0.0,"elapsed_seconds":-1,"width":0,"height":0})
}
fn merge(target: &mut Value, source: &Value) {
    if let (Some(target), Some(source)) = (target.as_object_mut(), source.as_object()) {
        target.extend(source.clone());
    }
}
fn checked_settings(value: &Value) -> Result<Value, String> {
    let object = value
        .as_object()
        .ok_or("Choose valid compression settings.")?;
    let mut compression = defaults();
    for (key, value) in object {
        if compression.get(key).is_some() {
            compression[key] = value.clone();
        }
    }
    let output = compression
        .as_object_mut()
        .unwrap()
        .remove("output_dir")
        .unwrap();
    let folder = output.as_str().ok_or("Choose an output folder.")?;
    let target = compression["target_mb"]
        .as_f64()
        .ok_or("Enter a size between 0.1 and 100000 MB.")?;
    if !(0.1..=100000.0).contains(&target) {
        return Err("Enter a size between 0.1 and 100000 MB.".into());
    }
    let mut checked = settings::validate(&compression)?;
    checked["output_dir"] = json!(folder);
    Ok(checked)
}

impl Controller {
    pub fn new(path: PathBuf) -> Arc<Self> {
        let mut inner = Inner {
            settings: defaults(),
            stored: json!({}),
            jobs: vec![],
            running: false,
            closed: false,
            stop: false,
            installing: false,
            error: String::new(),
            preview: json!({"status":"idle","job_id":"","percent":0,"stage":"","error":"","path":"","kind":"","source":"","size":0,"start_seconds":0,"duration_seconds":5,"settings":{}}),
            preview_temp: None,
            replacements: HashMap::new(),
        };
        if path.exists() {
            match fs::read(&path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                .filter(Value::is_object)
            {
                Some(stored) => {
                    if let Some(saved) = stored["settings"].as_object() {
                        for (key, value) in saved {
                            if inner.settings.get(key).is_some() {
                                let mut candidate = defaults();
                                candidate[key] = value.clone();
                                if let Ok(checked) = checked_settings(&candidate) {
                                    inner.settings[key] = checked[key].clone();
                                }
                            }
                        }
                    }
                    if let Some(jobs) = stored["jobs"].as_array() {
                        for item in jobs.iter().take(500) {
                            if let Some(source) = item["source"].as_str() {
                                if !matches!(
                                    item["status"].as_str(),
                                    Some(
                                        "pending"
                                            | "running"
                                            | "probing"
                                            | "completed"
                                            | "failed"
                                            | "cancelled"
                                    )
                                ) {
                                    continue;
                                }
                                let mut job = new_job(Path::new(source));
                                for key in [
                                    "id",
                                    "name",
                                    "kind",
                                    "original_size",
                                    "output",
                                    "output_size",
                                    "error",
                                    "status",
                                    "duration",
                                    "elapsed_seconds",
                                    "width",
                                    "height",
                                ] {
                                    if item.get(key).is_some()
                                        && ((job[key].is_string() && item[key].is_string())
                                            || (job[key].is_number() && item[key].is_number()))
                                    {
                                        job[key] = item[key].clone();
                                    }
                                }
                                if !terminal(&job) {
                                    merge(
                                        &mut job,
                                        &json!({"status":"pending","stage":"Ready to retry after restart"}),
                                    );
                                } else if job["status"] == "completed" {
                                    merge(
                                        &mut job,
                                        &json!({"percent":100,"stage":item["warning"].as_str().unwrap_or("Completed"),"warning":item["warning"]}),
                                    );
                                }
                                inner.jobs.push(job);
                            }
                        }
                    }
                    inner.stored = stored;
                }
                None => inner.error = "Saved settings could not be read. Using defaults.".into(),
            }
        }
        let controller = Arc::new(Self {
            inner: Mutex::new(inner),
            path,
            cancel: AtomicBool::new(false),
            preview_cancel: AtomicBool::new(false),
        });
        let owner = controller.clone();
        thread::spawn(move || {
            let jobs = owner.inner.lock().unwrap().jobs.clone();
            for job in jobs {
                if matches!(job["kind"].as_str(), Some("video" | "audio"))
                    && job["duration"].as_f64().unwrap_or(0.0) <= 0.0
                {
                    if let Ok(info) = engine::probe(Path::new(&text(&job, "source"))) {
                        let mut state = owner.inner.lock().unwrap();
                        if state.closed {
                            break;
                        }
                        if let Some(current) = state.jobs.iter_mut().find(|j| j["id"] == job["id"])
                        {
                            for key in ["duration", "width", "height"] {
                                if let Some(value) = info.get(key) {
                                    current[key] = value.clone();
                                }
                            }
                        }
                    }
                }
            }
        });
        controller
    }
    fn save(&self, inner: &mut Inner) {
        let result = (|| -> Result<(), String> {
            let parent = self.path.parent().ok_or("No state folder")?;
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            let mut saved = inner.stored.clone();
            if !saved["settings"].is_object() {
                saved["settings"] = json!({});
            }
            merge(&mut saved["settings"], &inner.settings);
            saved["jobs"] = json!(inner.jobs);
            let mut temp = NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
            temp.write_all(&serde_json::to_vec(&saved).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
            temp.as_file().sync_all().map_err(|e| e.to_string())?;
            replace_file(temp.path(), &self.path)
        })();
        if result.is_err() {
            inner.error =
                "Queue changes could not be saved. Check the app data folder permissions.".into();
        }
    }
    pub fn snapshot(&self) -> Value {
        let s = self.inner.lock().unwrap();
        json!({"settings":s.settings,"jobs":s.jobs,"running":s.running,"error":s.error,"preview":s.preview})
    }
    pub fn busy(&self) -> bool {
        let s = self.inner.lock().unwrap();
        s.running || s.preview["status"] == "running"
    }
    pub fn reserve_install(&self) -> bool {
        let mut s = self.inner.lock().unwrap();
        if s.running || s.preview["status"] == "running" || s.installing {
            false
        } else {
            s.installing = true;
            true
        }
    }
    pub fn release_install(&self) {
        self.inner.lock().unwrap().installing = false;
    }
    pub fn error(&self, error: impl Into<String>) {
        self.inner.lock().unwrap().error = error.into();
    }
    pub fn add_paths(self: &Arc<Self>, paths: Vec<String>) {
        let mut candidates = vec![];
        let mut skipped = 0;
        for value in paths {
            match fs::canonicalize(value) {
                Ok(path) if path.is_dir() => match fs::read_dir(path) {
                    Ok(entries) => {
                        let mut files: Vec<_> = entries
                            .flatten()
                            .map(|e| e.path())
                            .filter(|p| p.is_file())
                            .collect();
                        files.sort();
                        candidates.extend(files);
                    }
                    Err(_) => skipped += 1,
                },
                Ok(path) => candidates.push(path),
                Err(_) => skipped += 1,
            }
        }
        let mut added = vec![];
        {
            let mut s = self.inner.lock().unwrap();
            if s.closed {
                return;
            }
            s.error.clear();
            for path in candidates {
                if s.jobs.len() >= 500 {
                    s.error =
                        "The queue holds up to 500 files. Clear finished items before adding more."
                            .into();
                    break;
                }
                if !path.is_file() || !engine::supported_file(&path) {
                    skipped += 1;
                    continue;
                }
                if s.jobs.iter().any(|j| {
                    !terminal(j)
                        && same_file::is_same_file(Path::new(&text(j, "source")), &path)
                            .unwrap_or(false)
                }) {
                    continue;
                }
                let job = new_job(&path);
                added.push(job.clone());
                s.jobs.push(job);
            }
            if skipped > 0 && s.error.is_empty() {
                s.error = format!("{skipped} unsupported or unavailable file(s) were skipped.");
            }
            self.save(&mut s);
        }
        let owner = self.clone();
        thread::spawn(move || {
            for job in added {
                if owner.inner.lock().unwrap().closed {
                    break;
                }
                let result = engine::probe(Path::new(&text(&job, "source")));
                let mut s = owner.inner.lock().unwrap();
                if let Some(current) = s.jobs.iter_mut().find(|j| j["id"] == job["id"]) {
                    match result {
                        Ok(info) => {
                            merge(
                                current,
                                &json!({"kind":info["kind"],"original_size":info["size"],"status":"pending","stage":"Ready","duration":info["duration"].as_f64().unwrap_or(0.0),"width":info["width"].as_u64().unwrap_or(0),"height":info["height"].as_u64().unwrap_or(0)}),
                            );
                        }
                        Err(error) => merge(
                            current,
                            &json!({"status":"failed","stage":"Could not read file","error":error}),
                        ),
                    }
                }
                owner.save(&mut s);
            }
        });
    }
    pub fn update_settings(&self, patch: Value) {
        let mut s = self.inner.lock().unwrap();
        if s.running || s.preview["status"] == "running" || s.installing {
            s.error = "Stop compression or the preview before changing settings.".into();
            return;
        }
        if !patch.is_object() {
            s.error = "Choose valid compression settings.".into();
            return;
        }
        let mut candidate = s.settings.clone();
        merge(&mut candidate, &patch);
        match checked_settings(&candidate) {
            Ok(checked) => {
                let folder = text(&checked, "output_dir");
                if !folder.is_empty() && !Path::new(&folder).is_dir() {
                    s.error = "The output folder is unavailable. Choose another folder.".into();
                    return;
                }
                s.settings = checked;
                s.error.clear();
                self.save(&mut s);
            }
            Err(error) => s.error = error,
        }
    }
    pub fn start_queue(self: &Arc<Self>, only: Option<String>) {
        {
            let mut s = self.inner.lock().unwrap();
            if s.closed || s.running {
                return;
            }
            if s.installing || s.preview["status"] == "running" {
                s.error = "Finish the current operation before starting the queue.".into();
                return;
            }
            if !s.jobs.iter().any(|j| {
                matches!(j["status"].as_str(), Some("pending" | "probing"))
                    && only.as_ref().map_or(true, |id| j["id"] == *id)
            }) {
                s.error = "Add files or retry a failed item first.".into();
                return;
            }
            s.running = true;
            s.stop = false;
            s.error.clear();
            self.cancel.store(false, Ordering::SeqCst);
        }
        let owner = self.clone();
        thread::spawn(move || owner.run(only));
    }
    fn destination(source: &Path, kind: &str, options: &Value) -> Result<PathBuf, String> {
        let mut extension = text(options, &format!("{kind}_format"));
        if extension == "jpeg" {
            extension = "jpg".into();
        }
        if extension.is_empty() {
            return Err("Unsupported media type.".into());
        }
        let output = text(options, "output_dir");
        let folder = if output.is_empty() {
            source.parent().ok_or("No source folder")?.to_path_buf()
        } else {
            PathBuf::from(output)
        };
        if !folder.is_dir() {
            return Err("The output folder is unavailable.".into());
        }
        let stem: String = source
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .chars()
            .take(140)
            .collect();
        for number in 0..10000 {
            let suffix = if number == 0 {
                String::new()
            } else {
                format!(" ({})", number + 1)
            };
            let path = folder.join(format!("{stem}_compressed{suffix}.{extension}"));
            if fs::symlink_metadata(&path).is_err() {
                return Ok(path);
            }
        }
        Err("Too many copies share this filename. Choose another output folder.".into())
    }
    fn run(self: Arc<Self>, only: Option<String>) {
        loop {
            let (job, options, mut replacement) = {
                let mut s = self.inner.lock().unwrap();
                if s.stop || s.closed {
                    break;
                }
                let index = s.jobs.iter().position(|j| {
                    j["status"] == "pending" && only.as_ref().map_or(true, |id| j["id"] == *id)
                });
                let Some(index) = index else {
                    let probing = s.jobs.iter().any(|j| {
                        j["status"] == "probing" && only.as_ref().map_or(true, |id| j["id"] == *id)
                    });
                    drop(s);
                    if probing {
                        thread::sleep(Duration::from_millis(100));
                        continue;
                    }
                    break;
                };
                self.cancel.store(false, Ordering::SeqCst);
                merge(
                    &mut s.jobs[index],
                    &json!({"status":"running","stage":"Preparing","error":"","percent":0,"elapsed_seconds":-1}),
                );
                let job = s.jobs[index].clone();
                let replacement = s.replacements.remove(&text(&job, "id"));
                self.save(&mut s);
                (job, s.settings.clone(), replacement)
            };
            let started = Instant::now();
            let id = text(&job, "id");
            let replacing = replacement.is_some();
            let progress = |percent: f64, stage: &str| {
                let mut s = self.inner.lock().unwrap();
                if let Some(job) = s.jobs.iter_mut().find(|j| j["id"] == id) {
                    job["percent"] = json!(percent.clamp(0.0, 100.0));
                    job["stage"] = json!(stage);
                }
            };
            let result = (|| -> Result<Value, String> {
                let source = PathBuf::from(text(&job, "source"));
                let info = engine::probe(&source)?;
                {
                    let mut s = self.inner.lock().unwrap();
                    if let Some(current) = s.jobs.iter_mut().find(|j| j["id"] == id) {
                        current["kind"] = info["kind"].clone();
                        current["original_size"] = info["size"].clone();
                    }
                }
                if let Some((target, expected)) = replacement.take() {
                    let temp = tempfile::Builder::new()
                        .prefix(".compress-replace-")
                        .tempdir_in(target.parent().ok_or("No output folder")?)
                        .map_err(|e| e.to_string())?;
                    let staged = temp
                        .path()
                        .join(target.file_name().ok_or("No output filename")?);
                    let mut result =
                        engine::compress(&source, &staged, &options, &self.cancel, progress)
                            .map_err(|e| format!("Creating replacement: {e}"))?;
                    if self.cancel.load(Ordering::SeqCst) {
                        return Err("Cancelled".into());
                    }
                    if result["preserved_original"] == true {
                        result["path"] = json!(target.to_string_lossy());
                        result["size"] =
                            json!(fs::metadata(&target).map_err(|e| e.to_string())?.len());
                        result["warning"] = json!("Previous compressed copy kept: no smaller output passed the quality checks.");
                        return Ok(result);
                    }
                    if identity(&target).map_err(|e| format!("Checking previous output: {e}"))?
                        != expected
                    {
                        return Err("The previous output changed after confirmation. It was kept. Try again.".into());
                    }
                    drop(expected);
                    replace_file(&staged, &target)
                        .map_err(|e| format!("Replacing previous output: {e}"))?;
                    result["path"] = json!(target.to_string_lossy());
                    Ok(result)
                } else {
                    let destination = Self::destination(
                        &source,
                        info["kind"].as_str().unwrap_or_default(),
                        &options,
                    )?;
                    engine::compress(&source, &destination, &options, &self.cancel, progress)
                }
            })();
            let mut s = self.inner.lock().unwrap();
            if let Ok(result) = &result {
                if replacing && result["preserved_original"] != true {
                    for previous in &mut s.jobs {
                        if previous["output"] == result["path"] {
                            previous["output_size"] = result["size"].clone();
                            previous["stage"] = json!("Replaced by a later compression");
                        }
                    }
                }
            }
            if let Some(current) = s.jobs.iter_mut().find(|j| j["id"] == id) {
                match result {
                    Ok(result) => merge(
                        current,
                        &json!({"status":"completed","stage":result["warning"].as_str().unwrap_or("Completed"),"warning":result["warning"],"preserved_original":result["preserved_original"] == true,"percent":100,"output":result["path"],"output_size":result["size"],"elapsed_seconds":started.elapsed().as_secs()}),
                    ),
                    Err(error) => {
                        let cancelled = self.cancel.load(Ordering::SeqCst)
                            || error.eq_ignore_ascii_case("cancelled");
                        merge(
                            current,
                            &json!({"status":if cancelled{"cancelled"}else{"failed"},"stage":if cancelled{"Cancelled"}else{"Failed"},"percent":0,"error":if cancelled{String::new()}else{error}}),
                        );
                    }
                }
            }
            self.save(&mut s);
        }
        let mut s = self.inner.lock().unwrap();
        s.running = false;
        self.save(&mut s);
    }
    pub fn cancel_current(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }
    pub fn stop_queue(&self) {
        self.inner.lock().unwrap().stop = true;
        self.cancel_current();
    }
    pub fn cancel_preview(&self) {
        self.preview_cancel.store(true, Ordering::SeqCst);
    }
    pub fn mutate_job(&self, method: &str, id: &str) {
        let mut s = self.inner.lock().unwrap();
        match method {
            "remove_job" => {
                if s.preview["status"] == "running" && s.preview["job_id"] == id {
                    s.error = "Cancel the preview before removing this file.".into();
                    return;
                }
                s.jobs.retain(|j| j["id"] != id || j["status"] == "running");
            }
            "clear_finished" => {
                let preview_id = if s.preview["status"] == "running" {
                    text(&s.preview, "job_id")
                } else {
                    String::new()
                };
                s.jobs.retain(|j| {
                    !matches!(j["status"].as_str(), Some("completed" | "cancelled"))
                        || j["id"] == preview_id
                });
            }
            "retry_job" => {
                if let Some(job) = s.jobs.iter_mut().find(|j| {
                    j["id"] == id && matches!(j["status"].as_str(), Some("failed" | "cancelled"))
                }) {
                    merge(
                        job,
                        &json!({"status":"pending","error":"","percent":0,"stage":"Ready","output":"","output_size":0}),
                    );
                    s.error.clear();
                }
            }
            _ => {}
        }
        self.save(&mut s);
    }
    pub fn job_path(&self, id: &str, output: bool) -> Result<PathBuf, String> {
        let s = self.inner.lock().unwrap();
        let job = s
            .jobs
            .iter()
            .find(|j| j["id"] == id)
            .ok_or("Choose a queued file.")?;
        if output && job["status"] != "completed" {
            return Err("This item has no completed output.".into());
        }
        let path = PathBuf::from(text(job, if output { "output" } else { "source" }));
        if !path.is_file() {
            return Err("The file has been moved or deleted.".into());
        }
        Ok(path)
    }
    pub fn preview_path(&self) -> Result<PathBuf, String> {
        let s = self.inner.lock().unwrap();
        let path = PathBuf::from(text(&s.preview, "path"));
        if s.preview["status"] != "completed" || !path.is_file() {
            return Err("Create a preview first.".into());
        }
        Ok(path)
    }
    pub fn rerun(self: &Arc<Self>, id: &str, action: &str) {
        let result = (|| -> Result<String, String> {
            let mut s = self.inner.lock().unwrap();
            if s.closed || s.installing || s.running || s.preview["status"] == "running" {
                return Err(
                    "Finish the current operation before compressing this file again.".into(),
                );
            }
            if !matches!(action, "copy" | "replace") {
                return Err("Choose another copy or replace the previous output.".into());
            }
            let old = s
                .jobs
                .iter()
                .find(|j| j["id"] == id && j["status"] == "completed")
                .cloned()
                .ok_or("Choose a completed file to compress again.")?;
            if s.jobs.len() >= 500 {
                return Err("Clear finished items before adding another compression.".into());
            }
            if s.jobs
                .iter()
                .any(|j| j["source"] == old["source"] && !terminal(j))
            {
                return Err("This source already has a queued compression.".into());
            }
            let source = fs::canonicalize(text(&old, "source")).map_err(|e| e.to_string())?;
            let mut job = new_job(&source);
            for key in ["kind", "duration", "width", "height"] {
                job[key] = old[key].clone();
            }
            merge(
                &mut job,
                &json!({"original_size":fs::metadata(&source).map_err(|e|e.to_string())?.len(),"status":"pending","stage":"Ready"}),
            );
            let id = text(&job, "id");
            if action == "replace" {
                let target = PathBuf::from(text(&old, "output"));
                if same_file::is_same_file(&source, &target).map_err(|e| e.to_string())? {
                    return Err(
                        "The previous output points to the source. Choose another copy.".into(),
                    );
                }
                let format = text(&s.settings, &format!("{}_format", text(&old, "kind")));
                let extension = target
                    .extension()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_lowercase();
                if extension != format && !(format == "jpeg" && extension == "jpg") {
                    return Err(
                        "The output format changed. Choose another copy to use the new format."
                            .into(),
                    );
                }
                let fingerprint = identity(&target)?;
                s.replacements.insert(id.clone(), (target, fingerprint));
            }
            s.jobs.push(job);
            s.error.clear();
            self.save(&mut s);
            Ok(id)
        })();
        match result {
            Ok(id) => self.start_queue(Some(id)),
            Err(error) => self.error(error),
        }
    }
    pub fn start_preview(self: &Arc<Self>, id: &str, start: f64, duration: f64) {
        let result = (|| -> Result<(PathBuf, PathBuf, Value, f64), String> {
            let mut s = self.inner.lock().unwrap();
            if text(&s.settings, "compression_mode") == "auto" {
                return Err("Auto quality compares the full file. Switch to Size limit for a short preview.".into());
            }
            if s.closed || s.installing || s.running || s.preview["status"] == "running" {
                return Err("Finish the current operation before creating a preview.".into());
            }
            let job = s
                .jobs
                .iter()
                .find(|j| {
                    j["id"] == id
                        && matches!(j["kind"].as_str(), Some("video" | "audio" | "image"))
                        && j["status"] != "probing"
                })
                .cloned()
                .ok_or("Choose a file that has finished loading.")?;
            if !start.is_finite()
                || start < 0.0
                || !duration.is_finite()
                || !(1.0..=15.0).contains(&duration)
            {
                return Err(
                    "Preview start must be positive or zero. Duration must be 1 to 15 seconds."
                        .into(),
                );
            }
            let start = if job["kind"] == "image" { 0.0 } else { start };
            let length = job["duration"].as_f64().unwrap_or(0.0);
            if job["kind"] != "image" && length > 0.0 && start >= length {
                return Err("The preview start must be before the end of the file.".into());
            }
            let folder = self.path.parent().ok_or("No state folder")?;
            fs::create_dir_all(folder).map_err(|e| e.to_string())?;
            let temp = tempfile::Builder::new()
                .prefix("preview-")
                .tempdir_in(folder)
                .map_err(|e| e.to_string())?;
            let options = s.settings.clone();
            let extension = text(&options, &format!("{}_format", text(&job, "kind")));
            let destination = temp.path().join(format!("sample.{extension}"));
            s.preview_temp = Some(temp);
            self.preview_cancel.store(false, Ordering::SeqCst);
            s.preview = json!({"status":"running","job_id":id,"percent":0,"stage":"Preparing preview","error":"","path":"","size":0,"kind":job["kind"],"source":job["source"],"settings":options,"start_seconds":start,"duration_seconds":duration});
            s.error.clear();
            Ok((
                PathBuf::from(text(&job, "source")),
                destination,
                options,
                start,
            ))
        })();
        match result {
            Err(error) => self.error(error),
            Ok((source, destination, options, start)) => {
                let owner = self.clone();
                thread::spawn(move || {
                    let progress = |percent: f64, stage: &str| {
                        let mut s = owner.inner.lock().unwrap();
                        s.preview["percent"] = json!(percent.clamp(0.0, 100.0));
                        s.preview["stage"] = json!(stage);
                    };
                    let result = engine::preview(
                        &source,
                        &destination,
                        &options,
                        &owner.preview_cancel,
                        progress,
                        start,
                        duration,
                    );
                    let mut s = owner.inner.lock().unwrap();
                    if owner.preview_cancel.load(Ordering::SeqCst) {
                        merge(
                            &mut s.preview,
                            &json!({"status":"cancelled","path":"","percent":0,"stage":"Preview cancelled"}),
                        );
                    } else {
                        match result {
                            Ok(result) => {
                                merge(&mut s.preview, &result);
                                merge(
                                    &mut s.preview,
                                    &json!({"status":"completed","percent":100,"stage":"Preview ready"}),
                                );
                            }
                            Err(error) => merge(
                                &mut s.preview,
                                &json!({"status":"failed","path":"","percent":0,"stage":"Preview failed","error":error}),
                            ),
                        }
                    }
                    if s.closed {
                        s.preview_temp = None;
                    }
                });
            }
        }
    }
    pub fn close(&self) {
        {
            let mut s = self.inner.lock().unwrap();
            s.closed = true;
            s.stop = true;
            self.cancel_current();
            self.cancel_preview();
            self.save(&mut s);
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while self.busy() && std::time::Instant::now() < deadline {
            thread::sleep(Duration::from_millis(25));
        }
        let mut s = self.inner.lock().unwrap();
        if s.preview["status"] != "running" {
            s.preview_temp = None;
        }
        self.save(&mut s);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn restores_unknown_fields_and_completed_jobs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        fs::write(&path,r#"{"future":42,"settings":{"target_mb":25,"future_option":9},"jobs":[{"source":"missing.mp4","status":"completed","output":"old.mp4","elapsed_seconds":337},{"source":"legacy.mp4","status":"completed"}]}"#).unwrap();
        let owner = Controller::new(path.clone());
        assert_eq!(owner.snapshot()["jobs"][0]["percent"], 100);
        assert_eq!(owner.snapshot()["jobs"][0]["elapsed_seconds"], 337);
        assert_eq!(owner.snapshot()["jobs"][1]["elapsed_seconds"], -1);
        owner.update_settings(json!({"target_mb":10}));
        let saved: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert_eq!(saved["future"], 42);
        assert_eq!(saved["settings"]["future_option"], 9);
        assert_eq!(saved["jobs"][0]["elapsed_seconds"], 337);
    }
    #[test]
    fn invalid_settings_do_not_replace_previous() {
        let dir = tempfile::tempdir().unwrap();
        let owner = Controller::new(dir.path().join("state.json"));
        owner.update_settings(json!({"target_mb":-1}));
        assert_eq!(owner.snapshot()["settings"]["target_mb"], 10);
        assert!(!text(&owner.snapshot(), "error").is_empty());
    }
    #[test]
    fn installer_reservation_blocks_processing() {
        let dir = tempfile::tempdir().unwrap();
        let owner = Controller::new(dir.path().join("state.json"));
        assert!(owner.reserve_install());
        assert!(!owner.reserve_install());
        owner.start_queue(None);
        assert!(!owner.snapshot()["running"].as_bool().unwrap());
        owner.release_install();
        assert!(owner.reserve_install());
    }
    #[test]
    fn identity_detects_changed_output() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("old.mp4");
        fs::write(&path, b"old").unwrap();
        let original = identity(&path).unwrap();
        fs::write(&path, b"changed").unwrap();
        assert_ne!(original, identity(&path).unwrap());
    }
    #[test]
    fn destination_uses_numbered_copy() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("video.mp4");
        fs::write(dir.path().join("video_compressed.mp4"), b"old").unwrap();
        assert_eq!(
            Controller::destination(&source, "video", &defaults())
                .unwrap()
                .file_name()
                .unwrap(),
            "video_compressed (2).mp4"
        );
    }
    #[test]
    fn auto_destination_uses_selected_format() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("video.avi");
        let mut options = defaults();
        options["compression_mode"] = json!("auto");
        options["video_format"] = json!("mp4");
        assert_eq!(
            Controller::destination(&source, "video", &options)
                .unwrap()
                .extension()
                .unwrap(),
            "mp4"
        );
    }
    fn wait_for(owner: &Controller, predicate: impl Fn(&Value) -> bool) {
        let until = std::time::Instant::now() + Duration::from_secs(60);
        loop {
            let snapshot = owner.snapshot();
            if predicate(&snapshot) {
                return;
            }
            assert!(
                std::time::Instant::now() < until,
                "Operation timed out: {snapshot}"
            );
            thread::sleep(Duration::from_millis(25));
        }
    }
    fn fixture(folder: &Path) -> Option<PathBuf> {
        let tools = engine::find_tools();
        let Some(ffmpeg) = tools["ffmpeg"].as_str() else {
            return None;
        };
        let path = folder.join("source.mp4");
        let mut command = std::process::Command::new(ffmpeg);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let result = command
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=320x180:rate=24",
                "-t",
                "2",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
            ])
            .arg(&path)
            .status()
            .unwrap();
        assert!(result.success());
        Some(path)
    }
    #[test]
    fn auto_rerun_keeps_previous_copy_when_original_is_smallest() {
        let dir = tempfile::tempdir().unwrap();
        let tools = engine::find_tools();
        let Some(ffmpeg) = tools["ffmpeg"].as_str() else {
            return;
        };
        let source = dir.path().join("small.mp3");
        let status = std::process::Command::new(ffmpeg)
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=44100",
                "-t",
                "2",
                "-b:a",
                "8k",
            ])
            .arg(&source)
            .status()
            .unwrap();
        assert!(status.success());
        let owner = Controller::new(dir.path().join("state.json"));
        owner.add_paths(vec![source.to_string_lossy().into_owned()]);
        wait_for(&owner, |s| s["jobs"][0]["status"] == "pending");
        owner.update_settings(json!({"audio_format":"flac","target_mb":10}));
        owner.start_queue(None);
        wait_for(&owner, |s| s["running"] == false);
        let first = owner.snapshot()["jobs"][0].clone();
        assert_eq!(first["status"], "completed", "{first}");
        assert!(first["elapsed_seconds"].as_u64().is_some());
        let output = PathBuf::from(text(&first, "output"));
        let previous = fs::read(&output).unwrap();
        let id = text(&first, "id");
        owner.update_settings(json!({"compression_mode":"auto"}));
        owner.rerun(&id, "replace");
        wait_for(&owner, |s| s["running"] == false);
        let second = owner.snapshot()["jobs"][1].clone();
        assert_eq!(second["status"], "completed", "{second}");
        assert!(second["elapsed_seconds"].as_u64().is_some());
        assert_eq!(PathBuf::from(text(&second, "output")), output);
        assert_eq!(second["preserved_original"], true);
        assert_eq!(fs::read(&output).unwrap(), previous);
    }
    #[test]
    fn real_queue_preview_copy_replace_and_state_restore() {
        let dir = tempfile::tempdir().unwrap();
        let Some(source) = fixture(dir.path()) else {
            return;
        };
        let original = fs::read(&source).unwrap();
        let state_path = dir.path().join("state.json");
        let owner = Controller::new(state_path.clone());
        owner.add_paths(vec![source.to_string_lossy().into_owned()]);
        wait_for(&owner, |s| s["jobs"][0]["status"] == "pending");
        owner.update_settings(json!({"advanced_enabled":true,"rate_control":"bitrate","video_bitrate_kbps":300,"scale_percent":50,"encoder":"software"}));
        let id = text(&owner.snapshot()["jobs"][0], "id");
        owner.start_preview(&id, 0.5, 1.0);
        wait_for(&owner, |s| s["preview"]["status"] != "running");
        let preview = owner.snapshot()["preview"].clone();
        assert_eq!(preview["status"], "completed", "{preview}");
        assert_eq!(preview["width"], 160);
        owner.start_queue(None);
        wait_for(&owner, |s| s["running"] == false);
        let first = owner.snapshot()["jobs"][0].clone();
        assert_eq!(first["status"], "completed", "{first}");
        let output = PathBuf::from(text(&first, "output"));
        let first_data = fs::read(&output).unwrap();
        owner.rerun(&id, "copy");
        wait_for(&owner, |s| s["running"] == false);
        let second = owner.snapshot()["jobs"][1].clone();
        assert_eq!(second["status"], "completed", "{second}");
        assert_ne!(second["output"], first["output"]);
        assert_eq!(fs::read(&output).unwrap(), first_data);
        owner.update_settings(json!({"scale_percent":75}));
        owner.rerun(&id, "replace");
        wait_for(&owner, |s| s["running"] == false);
        let third = owner.snapshot()["jobs"][2].clone();
        assert_eq!(third["status"], "completed", "{third}");
        assert!(third["elapsed_seconds"].as_u64().is_some());
        assert_eq!(third["output"], first["output"]);
        assert_ne!(fs::read(&output).unwrap(), first_data);
        assert_eq!(fs::read(&source).unwrap(), original);
        owner.close();
        let restored = Controller::new(state_path);
        assert_eq!(restored.snapshot()["jobs"].as_array().unwrap().len(), 3);
        assert_eq!(restored.snapshot()["settings"]["scale_percent"], 75.0);
        assert_eq!(
            restored.snapshot()["jobs"][2]["elapsed_seconds"],
            third["elapsed_seconds"]
        );
        restored.close();
    }
    #[test]
    fn replacement_rejects_source_alias() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.mp4");
        fs::write(&source, b"source").unwrap();
        let owner = Controller::new(dir.path().join("state.json"));
        let mut job = new_job(&source);
        merge(
            &mut job,
            &json!({"status":"completed","kind":"video","output":source.to_string_lossy()}),
        );
        let id = text(&job, "id");
        owner.inner.lock().unwrap().jobs.push(job);
        owner.rerun(&id, "replace");
        assert!(text(&owner.snapshot(), "error").contains("source"));
        assert_eq!(fs::read(&source).unwrap(), b"source");
    }

    #[test]
    fn preview_reserves_settings_queue_and_installer() {
        let dir = tempfile::tempdir().unwrap();
        let owner = Controller::new(dir.path().join("state.json"));
        owner.inner.lock().unwrap().preview["status"] = json!("running");
        assert!(owner.busy());
        assert!(!owner.reserve_install());
        owner.update_settings(json!({"target_mb":25}));
        assert_eq!(owner.snapshot()["settings"]["target_mb"], 10);
        owner.start_queue(None);
        assert_eq!(owner.snapshot()["running"], false);
        owner.inner.lock().unwrap().preview["status"] = json!("cancelled");
        assert!(owner.reserve_install());
    }

    #[test]
    fn corrupt_state_and_invalid_fields_restore_independently() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        fs::write(&path, "broken json").unwrap();
        let owner = Controller::new(path.clone());
        assert!(!text(&owner.snapshot(), "error").is_empty());
        fs::write(&path,r#"{"settings":{"target_mb":25,"scale_percent":-1,"advanced_enabled":true},"jobs":[{"source":"missing.mp4","status":"running"}]}"#).unwrap();
        let restored = Controller::new(path);
        let state = restored.snapshot();
        assert_eq!(state["settings"]["target_mb"], 25.0);
        assert_eq!(state["settings"]["scale_percent"], 100);
        assert_eq!(state["settings"]["advanced_enabled"], true);
        assert_eq!(state["jobs"][0]["status"], "pending");
    }

    #[test]
    fn rerun_leaves_other_pending_files_and_failed_replace_keeps_output() {
        let dir = tempfile::tempdir().unwrap();
        let Some(source) = fixture(dir.path()) else {
            return;
        };
        let owner = Controller::new(dir.path().join("state.json"));
        owner.add_paths(vec![source.to_string_lossy().into_owned()]);
        wait_for(&owner, |s| s["jobs"][0]["status"] == "pending");
        owner.start_queue(None);
        wait_for(&owner, |s| s["running"] == false);
        let completed = owner.snapshot()["jobs"][0].clone();
        let id = text(&completed, "id");
        let output = PathBuf::from(text(&completed, "output"));
        let original = fs::read(&output).unwrap();
        let mut pending = new_job(&dir.path().join("missing.mp4"));
        pending["status"] = json!("pending");
        owner.inner.lock().unwrap().jobs.push(pending);
        owner.rerun(&id, "copy");
        wait_for(&owner, |s| s["running"] == false);
        assert_eq!(owner.snapshot()["jobs"][1]["status"], "pending");
        assert_eq!(owner.snapshot()["jobs"][2]["status"], "completed");
        fs::write(&source, b"invalid video").unwrap();
        owner.rerun(&id, "replace");
        wait_for(&owner, |s| s["running"] == false);
        assert_eq!(owner.snapshot()["jobs"][3]["status"], "failed");
        assert_eq!(fs::read(&output).unwrap(), original);
        owner.close();
    }

    #[test]
    fn cancelled_replacement_preserves_previous_file() {
        let dir = tempfile::tempdir().unwrap();
        let Some(source) = fixture(dir.path()) else {
            return;
        };
        let owner = Controller::new(dir.path().join("state.json"));
        owner.add_paths(vec![source.to_string_lossy().into_owned()]);
        wait_for(&owner, |s| s["jobs"][0]["status"] == "pending");
        owner.start_queue(None);
        wait_for(&owner, |s| s["running"] == false);
        let completed = owner.snapshot()["jobs"][0].clone();
        let output = PathBuf::from(text(&completed, "output"));
        let bytes = fs::read(&output).unwrap();
        owner.update_settings(json!({"advanced_enabled":true,"preset":"veryslow","encoder":"software","scale_percent":200}));
        owner.rerun(&text(&completed, "id"), "replace");
        wait_for(&owner, |s| s["jobs"][1]["status"] == "running");
        owner.cancel_current();
        wait_for(&owner, |s| s["running"] == false);
        assert_eq!(owner.snapshot()["jobs"][1]["status"], "cancelled");
        assert_eq!(fs::read(output).unwrap(), bytes);
        owner.close();
    }
}
