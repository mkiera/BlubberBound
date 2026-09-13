use reqwest::blocking::{Client, Response};
use semver::{BuildMetadata, Version};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use uuid::Uuid;

const MAX_INSTALLER: u64 = 300 * 1024 * 1024;
const MAX_ARCHIVE: u64 = 350 * 1024 * 1024;

fn text(value: &Value) -> &str {
    value.as_str().unwrap_or("")
}
fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}
fn version(value: &str) -> Option<Version> {
    let mut result = Version::parse(value.strip_prefix('v').unwrap_or(value)).ok()?;
    result.build = BuildMetadata::EMPTY;
    Some(result)
}
fn app_notes(value: &str) -> String {
    value
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .split("<!-- app-notes-end -->")
        .next()
        .unwrap_or("")
        .trim()
        .to_owned()
}
pub fn safe_url(value: &str) -> bool {
    if value.bytes().any(|c| c <= 32) {
        return false;
    }
    let Ok(url) = reqwest::Url::parse(value) else {
        return false;
    };
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.fragment().is_none()
        && url.port_or_known_default() == Some(443)
        && matches!(
            url.host_str(),
            Some(
                "github.com"
                    | "api.github.com"
                    | "objects.githubusercontent.com"
                    | "release-assets.githubusercontent.com"
                    | "nightly.link"
            )
        )
}
fn safe_installer(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name.as_bytes()[0].is_ascii_alphanumeric()
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
        && name.to_ascii_lowercase().ends_with("-setup.exe")
}
fn redirect_target(
    current: &reqwest::Url,
    location: &str,
    authenticated: bool,
) -> Result<(reqwest::Url, bool), SourceError> {
    if location.bytes().any(|c| c <= 32) {
        return Err("The update redirect is invalid.".into());
    }
    let next = current
        .join(location)
        .map_err(|_| SourceError::from("The update redirect is invalid."))?;
    if !safe_url(next.as_str()) {
        return Err("The update server redirected to an untrusted address.".into());
    }
    let authenticated = authenticated && current.host_str() == next.host_str();
    Ok((next, authenticated))
}
fn asset_valid(asset: &Value, profile: &Value) -> bool {
    let name = text(&asset["name"]);
    safe_installer(name)
        && name.eq_ignore_ascii_case(text(&profile["installer_asset"]))
        && safe_url(text(
            asset.get("browser_download_url").unwrap_or(&asset["url"]),
        ))
        && asset["size"]
            .as_u64()
            .is_some_and(|v| v > 0 && v <= MAX_INSTALLER)
}
fn page(profile: &Value) -> String {
    format!(
        "https://github.com/{}/{}",
        text(&profile["owner"]),
        text(&profile["repository"])
    )
}
fn api(profile: &Value) -> String {
    format!(
        "https://api.github.com/repos/{}/{}",
        text(&profile["owner"]),
        text(&profile["repository"])
    )
}

pub fn release_rows(records: &Value, channel: &str, running: &str, profile: &Value) -> Vec<Value> {
    let current = version(running);
    let mut rows = Vec::new();
    for record in records.as_array().into_iter().flatten() {
        if record["draft"].as_bool() == Some(true) || record["id"].as_u64().unwrap_or(0) == 0 {
            continue;
        }
        let Some(candidate) = version(text(&record["tag_name"])) else {
            continue;
        };
        let prerelease = !candidate.pre.is_empty() || record["prerelease"].as_bool() == Some(true);
        if channel != "prerelease" && prerelease {
            continue;
        }
        let Some(asset) = record["assets"]
            .as_array()
            .and_then(|items| items.iter().find(|a| asset_valid(a, profile)))
        else {
            continue;
        };
        let action = match current.as_ref().map(|v| candidate.cmp(v)) {
            None => "Install",
            Some(std::cmp::Ordering::Greater) => "Update",
            Some(std::cmp::Ordering::Less) => "Downgrade",
            _ => "Reinstall",
        };
        let raw_version = text(&record["tag_name"]).trim_start_matches('v');
        let url = if safe_url(text(&record["html_url"])) {
            text(&record["html_url"]).to_owned()
        } else {
            format!("{}/releases/tag/v{}", page(profile), raw_version)
        };
        rows.push(json!({"id":format!("release:{}",record["id"]),"release_id":record["id"],"version":raw_version,
            "title":raw_version,"subtitle":format!("{}{}, {:.1} MB",if prerelease {"Pre-release, "} else {""},
                text(&record["published_at"]).split('T').next().unwrap_or("Date unavailable"),asset["size"].as_u64().unwrap_or(0) as f64 / 1_000_000.0),
            "notes":app_notes(text(&record["body"])),"url":url,"action":action,"running":current.as_ref()==Some(&candidate),
            "kind":"release","prerelease":prerelease,"asset":asset}));
    }
    rows.sort_by(|a, b| version(text(&b["version"])).cmp(&version(text(&a["version"]))));
    rows
}

pub fn alpha_rows(runs: &Value, branches: &Value, identity: &Value, profile: &Value) -> Vec<Value> {
    let live: HashSet<String> = branches
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().or(v["name"].as_str()))
        .filter(|s| !s.trim().is_empty())
        .map(str::to_lowercase)
        .collect();
    let mut newest: HashMap<String, &Value> = HashMap::new();
    for run in runs.as_array().into_iter().flatten() {
        let branch = text(&run["head_branch"]);
        if text(&run["conclusion"]) != "success"
            || run["id"].as_u64().unwrap_or(0) == 0
            || run["run_number"].as_u64().unwrap_or(0) == 0
            || branch.trim().is_empty()
            || (!live.is_empty() && !live.contains(&branch.to_lowercase()))
        {
            continue;
        }
        let key = branch.to_lowercase();
        let order = |v: &Value| {
            (
                v["run_number"].as_u64().unwrap_or(0),
                v["id"].as_u64().unwrap_or(0),
            )
        };
        if newest.get(&key).is_none_or(|old| order(run) > order(old)) {
            newest.insert(key, run);
        }
    }
    let mut runs: Vec<_> = newest.values().copied().collect();
    runs.sort_by_key(|r| std::cmp::Reverse((r["run_number"].as_u64(), r["id"].as_u64())));
    runs.into_iter().take(8).map(|r| {
        let run_id = r["id"].as_u64().unwrap_or(0);
        let sha = text(&r["head_sha"]);
        let current = text(&identity["run_id"]);
        let running = if !current.is_empty() { current==run_id.to_string() } else { !text(&identity["commit"]).is_empty() && sha.eq_ignore_ascii_case(text(&identity["commit"])) };
        json!({"id":format!("alpha:{run_id}"),"run_id":run_id,"run_number":r["run_number"],"branch":r["head_branch"],"sha":sha,
            "title":format!("{} #{}",text(&r["head_branch"]),r["run_number"]),"subtitle":format!("commit {}, {}",sha.chars().take(7).collect::<String>(),text(&r["created_at"])),
            "url":format!("{}/actions/runs/{run_id}",page(profile)),"action":"Install","running":running,"kind":"alpha"})
    }).collect()
}

pub fn notes_since(records: &Value, previous: &str, running: &str) -> Vec<Value> {
    let (Some(previous), Some(current)) = (version(previous), version(running)) else {
        return vec![];
    };
    if current <= previous {
        return vec![];
    }
    let mut rows = Vec::new();
    for record in records.as_array().into_iter().flatten() {
        let Some(v) = version(text(&record["tag_name"])) else {
            continue;
        };
        if record["draft"].as_bool() == Some(true)
            || v <= previous
            || v > current
            || (current.pre.is_empty()
                && (!v.pre.is_empty() || record["prerelease"].as_bool() == Some(true)))
        {
            continue;
        }
        let notes = app_notes(text(&record["body"]));
        if !notes.is_empty() {
            rows.push(
                json!({"version":text(&record["tag_name"]).trim_start_matches('v'),"notes":notes}),
            );
        }
    }
    rows.sort_by(|a, b| version(text(&b["version"])).cmp(&version(text(&a["version"]))));
    rows
}

fn hidden(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command.stdin(Stdio::null()).stderr(Stdio::null());
}
fn local_token() -> Option<String> {
    let mut token = std::env::var("BLUBBERBOUND_GITHUB_TOKEN")
        .unwrap_or_default()
        .trim()
        .to_owned();
    if token.is_empty() {
        let mut command = Command::new("gh");
        hidden(&mut command);
        if let Ok(mut child) = command
            .args(["auth", "token", "--hostname", "github.com"])
            .stdout(Stdio::piped())
            .spawn()
        {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                if let Ok(Some(status)) = child.try_wait() {
                    if status.success() {
                        if let Some(stdout) = child.stdout.take() {
                            let _ = stdout.take(16384).read_to_string(&mut token);
                        }
                    }
                    break;
                }
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    break;
                }
                thread::sleep(Duration::from_millis(25));
            }
        }
    }
    let token = token.trim().to_owned();
    if token.is_empty() || token.contains(['\r', '\n']) {
        None
    } else {
        Some(token)
    }
}

#[derive(Debug)]
struct SourceError {
    message: String,
    code: Option<u16>,
}
impl From<String> for SourceError {
    fn from(message: String) -> Self {
        Self {
            message,
            code: None,
        }
    }
}
impl From<&str> for SourceError {
    fn from(message: &str) -> Self {
        message.to_owned().into()
    }
}
struct Source {
    profile: Value,
    client: Client,
    private: Option<bool>,
    token: Option<String>,
    version: String,
}
impl Source {
    fn new(profile: Value, version: String) -> Self {
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(120))
            .build()
            .expect("HTTPS client initialization failed");
        Self {
            profile,
            version,
            client,
            private: None,
            token: None,
        }
    }
    fn request(
        &mut self,
        url: &str,
        authenticated: bool,
        binary: bool,
    ) -> Result<Response, SourceError> {
        if !safe_url(url) {
            return Err("The update address is not trusted.".into());
        }
        let mut url =
            reqwest::Url::parse(url).map_err(|_| SourceError::from("Invalid update address."))?;
        let mut auth = authenticated;
        if auth && url.host_str() != Some("api.github.com") {
            return Err("Authentication is restricted to the GitHub API.".into());
        }
        if auth && self.token.is_none() {
            self.token = local_token();
        }
        if auth && self.token.is_none() {
            return Err("This repository is private. Sign in with GitHub CLI or set BLUBBERBOUND_GITHUB_TOKEN to an account with repository access.".into());
        }
        for _ in 0..8 {
            let mut request = self
                .client
                .get(url.clone())
                .timeout(Duration::from_secs(if binary { 120 } else { 30 }))
                .header(
                    "User-Agent",
                    format!("{}/{}", text(&self.profile["display_name"]), self.version),
                )
                .header(
                    "Accept",
                    if binary {
                        "application/octet-stream"
                    } else {
                        "application/vnd.github+json"
                    },
                )
                .header("X-GitHub-Api-Version", "2022-11-28");
            if auth {
                request = request.bearer_auth(self.token.as_ref().unwrap());
            }
            let response = request.send().map_err(|_| {
                SourceError::from(
                    "GitHub could not be reached. Check your connection and try again.",
                )
            })?;
            if response.status().is_redirection() {
                let location = response
                    .headers()
                    .get("location")
                    .and_then(|v| v.to_str().ok())
                    .ok_or_else(|| SourceError::from("The update redirect is invalid."))?;
                (url, auth) = redirect_target(&url, location, auth)?;
                continue;
            }
            let code = response.status().as_u16();
            if !response.status().is_success() {
                let message = if code == 403
                    && response
                        .headers()
                        .get("X-RateLimit-Remaining")
                        .is_some_and(|v| v == "0")
                {
                    "GitHub rate limit reached. Try again later.".to_owned()
                } else if matches!(code, 401 | 403 | 404) {
                    "GitHub could not grant access to this repository or update. Check your account access and try again.".to_owned()
                } else {
                    format!("GitHub returned HTTP {code}. Try again later.")
                };
                return Err(SourceError {
                    message,
                    code: Some(code),
                });
            }
            return Ok(response);
        }
        Err("The update server redirected too many times.".into())
    }
    fn json(&mut self, url: &str, authenticated: bool) -> Result<Value, SourceError> {
        let response = self.request(url, authenticated, false)?;
        let mut bytes = Vec::new();
        response
            .take(8 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| SourceError::from("Reading the update list failed. Try again."))?;
        if bytes.len() > 8 * 1024 * 1024 {
            return Err("GitHub returned an unexpectedly large update list.".into());
        }
        serde_json::from_slice(&bytes)
            .map_err(|_| "GitHub returned an unreadable update list. Try again later.".into())
    }
    fn discover(&mut self) -> Result<bool, SourceError> {
        if let Some(private) = self.private {
            return Ok(private);
        }
        let endpoint = api(&self.profile);
        let info = match self.json(&endpoint, false) {
            Ok(v) => v,
            Err(e) if matches!(e.code, Some(401 | 403 | 404)) => self.json(&endpoint, true)?,
            Err(e) => return Err(e),
        };
        let private = info["private"].as_bool().ok_or_else(|| {
            SourceError::from("GitHub returned unreadable repository information.")
        })?;
        self.private = Some(private);
        Ok(private)
    }
    fn records(&mut self, alpha: bool) -> Result<Value, SourceError> {
        self.private = None;
        let private = self.discover()?;
        let endpoint = api(&self.profile);
        if !alpha {
            let records = self.json(&format!("{endpoint}/releases?per_page=100"), private)?;
            if !records.is_array() {
                return Err("GitHub returned an unreadable release list.".into());
            }
            return Ok(records);
        }
        let workflow = text(&self.profile["alpha_workflow"]);
        let url =
            format!("{endpoint}/actions/workflows/{workflow}/runs?status=success&per_page=50");
        let data = match self.json(&url, private) {
            Ok(v) => v,
            Err(e) if e.code == Some(404) => json!({"workflow_runs":[]}),
            Err(e) => return Err(e),
        };
        if !data["workflow_runs"].is_array() {
            return Err("GitHub returned an unreadable branch build list.".into());
        }
        let branches = self
            .json(&format!("{endpoint}/branches?per_page=100"), private)
            .unwrap_or(Value::Null);
        Ok(json!({"runs":data["workflow_runs"],"branches":branches}))
    }
    fn download_stream(
        &mut self,
        url: &str,
        path: &Path,
        expected: Option<u64>,
        ceiling: u64,
        auth: bool,
        cancel: &AtomicBool,
        progress: &impl Fn(Value),
    ) -> Result<(), String> {
        let response = self.request(url, auth, true).map_err(|e| e.message)?;
        if response.content_length().is_some_and(|n| n > ceiling) {
            return Err("The downloaded update exceeds the size limit.".into());
        }
        copy_download(response, path, expected, ceiling, cancel, progress)
    }
    fn download(
        &mut self,
        row: &Value,
        folder: &Path,
        cancel: &AtomicBool,
        progress: impl Fn(Value),
    ) -> Result<PathBuf, String> {
        if cancel.load(Ordering::Relaxed) {
            return Err("Update cancelled.".into());
        }
        if row["kind"] == "release" && !asset_valid(&row["asset"], &self.profile) {
            return Err("The selected installer is not trusted.".into());
        }
        if row["kind"] != "release"
            && (row["kind"] != "alpha" || row["run_id"].as_u64().unwrap_or(0) == 0)
        {
            return Err("Choose a valid update.".into());
        }
        let private = self.discover().map_err(|e| e.message)?;
        fs::create_dir_all(folder).map_err(|e| e.to_string())?;
        let name = format!(
            "{}-{}-Setup.exe",
            text(&row["kind"]),
            Uuid::new_v4().simple()
        );
        let destination = folder.join(&name);
        let partial = folder.join(format!(".{name}.part"));
        let archive = folder.join(format!(".{name}.zip"));
        let result = (|| {
            if row["kind"] == "release" {
                let asset = &row["asset"];
                if !asset_valid(asset, &self.profile) {
                    return Err("The selected installer is not trusted.".into());
                }
                let url = if private {
                    let id = asset["id"]
                        .as_u64()
                        .filter(|n| *n > 0)
                        .ok_or("The private release has no valid asset ID.")?;
                    format!("{}/releases/assets/{id}", api(&self.profile))
                } else {
                    text(asset.get("browser_download_url").unwrap_or(&asset["url"])).to_owned()
                };
                self.download_stream(
                    &url,
                    &partial,
                    asset["size"].as_u64(),
                    MAX_INSTALLER,
                    private,
                    cancel,
                    &progress,
                )?;
            } else if row["kind"] == "alpha" {
                let id = row["run_id"]
                    .as_u64()
                    .filter(|n| *n > 0)
                    .ok_or("The selected branch build is invalid.")?;
                let url = if private {
                    let data = self
                        .json(
                            &format!(
                                "{}/actions/runs/{id}/artifacts?per_page=100",
                                api(&self.profile)
                            ),
                            true,
                        )
                        .map_err(|e| e.message)?;
                    let artifact=data["artifacts"].as_array().ok_or("GitHub returned an unreadable artifact list.")?.iter().find(|v|v["name"]==self.profile["alpha_artifact"] && v["expired"]!=true && v["id"].as_u64().unwrap_or(0)>0).ok_or("This branch artifact has expired or is unavailable. Choose a newer build.")?;
                    format!(
                        "{}/actions/artifacts/{}/zip",
                        api(&self.profile),
                        artifact["id"]
                    )
                } else {
                    format!(
                        "https://nightly.link/{}/{}/actions/runs/{id}/{}.zip",
                        text(&self.profile["owner"]),
                        text(&self.profile["repository"]),
                        text(&self.profile["alpha_artifact"])
                    )
                };
                self.download_stream(
                    &url,
                    &archive,
                    None,
                    MAX_ARCHIVE,
                    private,
                    cancel,
                    &progress,
                )?;
                extract_installer(
                    &archive,
                    &partial,
                    text(&self.profile["installer_asset"]),
                    cancel,
                    &progress,
                )?;
            } else {
                return Err("Choose a valid update.".into());
            }
            validate_binary(&partial)?;
            if cancel.load(Ordering::Relaxed) {
                return Err("Update cancelled.".into());
            }
            fs::rename(&partial, &destination).map_err(|e| e.to_string())?;
            progress(json!({"percent":100,"status":"Update downloaded."}));
            Ok(destination)
        })();
        let _ = fs::remove_file(partial);
        let _ = fs::remove_file(archive);
        result
    }
}

fn copy_download(
    mut source: impl Read,
    path: &Path,
    expected: Option<u64>,
    ceiling: u64,
    cancel: &AtomicBool,
    progress: &impl Fn(Value),
) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) {
        return Err("Update download cancelled.".into());
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    let result = (|| {
        let mut block = [0u8; 65536];
        let mut total = 0u64;
        let started = Instant::now();
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err("Update download cancelled.".into());
            }
            if started.elapsed() > Duration::from_secs(600) {
                return Err("Update download timed out. Try again.".into());
            }
            let n = source
                .read(&mut block)
                .map_err(|_| "The update download failed. Try again.".to_owned())?;
            if n == 0 {
                break;
            }
            total += n as u64;
            if total > ceiling || expected.is_some_and(|size| total > size) {
                return Err("The downloaded update is larger than its expected size.".into());
            }
            file.write_all(&block[..n]).map_err(|e| e.to_string())?;
            progress(
                json!({"percent":expected.filter(|n|*n>0).map(|n|total*99/n).unwrap_or(0),"status":format!("Downloading update: {:.1} MB",total as f64/1_000_000.0)}),
            );
        }
        if cancel.load(Ordering::Relaxed) {
            return Err("Update download cancelled.".into());
        }
        if total == 0 || expected.is_some_and(|size| total != size) {
            return Err("The update download was incomplete. Try again.".into());
        }
        file.sync_all().map_err(|e| e.to_string())
    })();
    drop(file);
    if result.is_err() {
        let _ = fs::remove_file(path);
    }
    result
}

fn extract_installer(
    archive: &Path,
    destination: &Path,
    name: &str,
    cancel: &AtomicBool,
    progress: &impl Fn(Value),
) -> Result<(), String> {
    let file = File::open(archive).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|_| "The branch archive is damaged or unreadable.".to_owned())?;
    for index in 0..zip.len() {
        let mut entry = zip
            .by_index(index)
            .map_err(|_| "The branch archive is damaged or unreadable.".to_owned())?;
        if !safe_installer(entry.name())
            || !entry.name().eq_ignore_ascii_case(name)
            || entry.is_dir()
        {
            continue;
        }
        if entry.size() == 0
            || entry.size() > MAX_INSTALLER
            || entry
                .unix_mode()
                .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err(
                "The branch archive contains no valid installer within the size limit.".into(),
            );
        }
        let expected = entry.size();
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(destination)
            .map_err(|e| e.to_string())?;
        let mut buffer = [0u8; 65536];
        let mut total = 0u64;
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err("Update cancelled.".into());
            }
            let n = entry
                .read(&mut buffer)
                .map_err(|_| "The branch archive is damaged or unreadable.".to_owned())?;
            if n == 0 {
                break;
            }
            total += n as u64;
            if total > MAX_INSTALLER || total > expected {
                return Err("The extracted installer exceeds the size limit.".into());
            }
            output.write_all(&buffer[..n]).map_err(|e| e.to_string())?;
            progress(json!({"percent":total*99/expected,"status":"Preparing branch installer"}));
        }
        if total != expected {
            return Err("The branch installer is incomplete.".into());
        }
        return output.sync_all().map_err(|e| e.to_string());
    }
    Err("The branch archive contains no valid installer within the size limit.".into())
}
fn validate_binary(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() == 0
        || metadata.len() > MAX_INSTALLER
    {
        return Err("The installer size or file type is invalid.".into());
    }
    let mut signature = [0u8; 2];
    File::open(path)
        .and_then(|mut f| f.read_exact(&mut signature))
        .map_err(|_| "The downloaded file is not a Windows installer.".to_owned())?;
    if signature != *b"MZ" {
        return Err("The downloaded file is not a Windows installer.".into());
    }
    Ok(())
}
fn pending_prompt(text: &str) -> bool {
    let mut pending = false;
    for line in text.lines().map(str::to_lowercase) {
        if line.contains("user chose")
            || line.contains("user selected")
            || line.contains("message box returned")
        {
            pending = false;
        } else if line.contains("message box")
            || line.contains("messagebox")
            || line.contains("waiting for user")
        {
            pending = true;
        }
    }
    pending
}
fn installer_path(path: &Path, folder: &Path) -> Result<(PathBuf, PathBuf), String> {
    validate_binary(path)?;
    let folder = folder.canonicalize().map_err(|e| e.to_string())?;
    let path = path.canonicalize().map_err(|e| e.to_string())?;
    if !path.starts_with(&folder)
        || !safe_installer(path.file_name().and_then(|n| n.to_str()).unwrap_or(""))
    {
        return Err("The installer must be inside the updates folder.".into());
    }
    Ok((path, folder))
}
fn launch_installer(path: &Path, folder: &Path) -> Result<bool, String> {
    if !cfg!(windows) {
        return Err("Automatic installation is available for Windows builds. Open the release page for other systems.".into());
    }
    let (path, folder) = installer_path(path, folder)?;
    let log = folder.join(format!("update-{}.log", Uuid::new_v4().simple()));
    let mut command = Command::new(&path);
    hidden(&mut command);
    let mut child = command
        .args(["/SILENT", "/CLOSEAPPLICATIONS", "/NORESTARTAPPLICATIONS"])
        .arg(format!("/LOG={}", log.display()))
        .current_dir(&folder)
        .stdout(Stdio::null())
        .spawn()
        .map_err(|_| "The installer could not start. The current app will stay open.".to_owned())?;
    wait_for_installer(
        || {
            child
                .try_wait()
                .map(|status| {
                    status.map(|status| {
                        if status.success() {
                            0
                        } else {
                            status.code().unwrap_or(-1)
                        }
                    })
                })
                .map_err(|e| e.to_string())
        },
        &log,
        Duration::from_millis(1500),
    )
}
fn wait_for_installer(
    mut poll: impl FnMut() -> Result<Option<i32>, String>,
    log: &Path,
    timeout: Duration,
) -> Result<bool, String> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(code) = poll()? {
            return if code == 0 {
                Ok(true)
            } else {
                Err(format!(
                    "The installer stopped with code {}. The current app will stay open.",
                    code
                ))
            };
        }
        if Instant::now() >= deadline {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
    startup_log(&log)
}
fn startup_log(log: &Path) -> Result<bool, String> {
    let mut raw = Vec::new();
    File::open(log).and_then(|file|file.take(4 * 1024 * 1024 + 1).read_to_end(&mut raw)).map_err(|_|"The installer has not confirmed startup. Complete any installer prompt before closing the app.".to_owned())?;
    if raw.len() > 4 * 1024 * 1024 {
        return Err(
            "The installer status is unexpectedly large. The current app will stay open.".into(),
        );
    }
    let decoded = if raw.starts_with(&[255, 254]) {
        String::from_utf16_lossy(
            &raw[2..]
                .chunks_exact(2)
                .map(|v| u16::from_le_bytes([v[0], v[1]]))
                .collect::<Vec<_>>(),
        )
    } else if raw.starts_with(&[254, 255]) {
        String::from_utf16_lossy(
            &raw[2..]
                .chunks_exact(2)
                .map(|v| u16::from_be_bytes([v[0], v[1]]))
                .collect::<Vec<_>>(),
        )
    } else {
        String::from_utf8_lossy(&raw).into_owned()
    };
    if decoded.trim().is_empty() || pending_prompt(&decoded) {
        return Err("The installer is waiting for a response. Complete its prompt while the current app stays open.".into());
    }
    Ok(true)
}

struct State {
    preferences: Value,
    rows: Vec<Value>,
    offer: Value,
    dismissed: Value,
    notes: Value,
    status: String,
    checking: bool,
    pending: bool,
    armed: Value,
    download: Value,
    downloaded: HashMap<String, PathBuf>,
    ready: Option<PathBuf>,
}
struct Inner {
    profile: Value,
    identity: Mutex<Value>,
    folder: PathBuf,
    state: Mutex<State>,
    source: Mutex<Source>,
    cache: Mutex<HashMap<String, (f64, Value)>>,
    closed: AtomicBool,
    cancel: AtomicBool,
}
#[derive(Clone)]
pub struct Updater {
    inner: Arc<Inner>,
}
impl Updater {
    pub fn new(profile: Value, data_dir: PathBuf, running: String) -> Self {
        let mut preferences = fs::read(data_dir.join("updates.json"))
            .ok()
            .and_then(|v| serde_json::from_slice::<Value>(&v).ok())
            .filter(Value::is_object)
            .unwrap_or(json!({}));
        if !matches!(
            text(&preferences["channel"]),
            "stable" | "prerelease" | "alpha"
        ) {
            preferences["channel"] = json!("stable");
        }
        if !preferences["automatic"].is_boolean() {
            preferences["automatic"] = json!(true);
        }
        if !preferences["checked_at"].is_number() {
            preferences["checked_at"] = json!(0);
        }
        if text(&preferences["last_seen"]).is_empty() {
            preferences["last_seen"] = json!(running);
        }
        let identity = json!({"version":running,"commit":"","branch":"","run_id":""});
        let updater = Self {
            inner: Arc::new(Inner {
                profile: profile.clone(),
                identity: Mutex::new(identity),
                folder: data_dir,
                source: Mutex::new(Source::new(profile, running)),
                cache: Mutex::new(HashMap::new()),
                closed: AtomicBool::new(false),
                cancel: AtomicBool::new(false),
                state: Mutex::new(State {
                    preferences,
                    rows: vec![],
                    offer: Value::Null,
                    dismissed: Value::Null,
                    notes: Value::Null,
                    status: String::new(),
                    checking: false,
                    pending: false,
                    armed: Value::Null,
                    download: json!({"active":false,"percent":0,"status":""}),
                    downloaded: HashMap::new(),
                    ready: None,
                }),
            }),
        };
        updater.save(&mut updater.inner.state.lock().unwrap());
        updater
    }
    pub fn set_identity(&self, identity: Value) {
        *self.inner.identity.lock().unwrap() = identity;
    }
    pub fn start(&self) {
        let weak = Arc::downgrade(&self.inner);
        thread::spawn(move || {
            thread::sleep(Duration::from_secs(4));
            let mut initial = true;
            let mut last_attempt = None::<Instant>;
            loop {
                let Some(inner) = weak.upgrade() else {
                    break;
                };
                if inner.closed.load(Ordering::Relaxed) {
                    break;
                }
                let updater = Self { inner };
                if initial {
                    updater.load_notes();
                    initial = false;
                }
                let due = {
                    let s = updater.inner.state.lock().unwrap();
                    s.preferences["automatic"] == true
                        && s.preferences["channel"] != "alpha"
                        && now() - s.preferences["checked_at"].as_f64().unwrap_or(0.0) >= 3600.0
                };
                if due
                    && last_attempt.is_none_or(|time| time.elapsed() >= Duration::from_secs(3600))
                {
                    updater.check(true, false);
                    last_attempt = Some(Instant::now());
                }
                drop(updater);
                thread::sleep(Duration::from_secs(30));
            }
        });
    }
    fn save(&self, state: &mut State) {
        let result = (|| -> Result<(), String> {
            fs::create_dir_all(&self.inner.folder).map_err(|e| e.to_string())?;
            let mut temporary =
                tempfile::NamedTempFile::new_in(&self.inner.folder).map_err(|e| e.to_string())?;
            serde_json::to_writer(&mut temporary, &state.preferences).map_err(|e| e.to_string())?;
            temporary.as_file().sync_all().map_err(|e| e.to_string())?;
            temporary
                .persist(self.inner.folder.join("updates.json"))
                .map_err(|e| e.to_string())?;
            Ok(())
        })();
        if result.is_err() {
            state.status = "Update preferences could not be saved.".into();
        }
    }
    pub fn snapshot(&self) -> Value {
        let s = self.inner.state.lock().unwrap();
        json!({"identity":self.inner.identity.lock().unwrap().clone(),"branding":self.inner.profile,"channel":s.preferences["channel"],"automatic":s.preferences["automatic"],
            "checked_at":s.preferences["checked_at"],"checking":s.checking,"status":s.status,"rows":s.rows,"offer":s.offer,"download":s.download,"armed_id":s.armed,"whats_new":s.notes})
    }
    fn records(&self, alpha: bool, force: bool) -> Result<Value, String> {
        let key = if alpha { "alpha" } else { "releases" };
        let mut source = self.inner.source.lock().unwrap();
        if !force {
            if let Some((time, data)) = self.inner.cache.lock().unwrap().get(key) {
                if now() - time < 300.0 {
                    return Ok(data.clone());
                }
            }
        }
        let data = source.records(alpha).map_err(|e| e.message)?;
        self.inner
            .cache
            .lock()
            .unwrap()
            .insert(key.to_owned(), (now(), data.clone()));
        Ok(data)
    }
    fn check(&self, quiet: bool, force: bool) {
        let (channel, previous) = {
            let mut s = self.inner.state.lock().unwrap();
            if self.inner.closed.load(Ordering::Relaxed)
                || (quiet
                    && (s.preferences["automatic"] != true || s.preferences["channel"] == "alpha"))
            {
                return;
            }
            if s.checking {
                if !quiet {
                    s.pending = true;
                }
                return;
            }
            if !quiet {
                s.dismissed = Value::Null;
                s.status = "Checking GitHub...".into();
            }
            s.armed = Value::Null;
            s.checking = true;
            let previous = s.preferences["checked_at"].clone();
            s.preferences["checked_at"] = json!(now());
            (text(&s.preferences["channel"]).to_owned(), previous)
        };
        let this = self.clone();
        thread::spawn(move || {
            let records = this.records(channel == "alpha", force);
            let identity = this.inner.identity.lock().unwrap().clone();
            let mut s = this.inner.state.lock().unwrap();
            if s.preferences["channel"] != channel {
                s.pending = true;
            } else {
                match records {
                    Ok(records) => {
                        s.rows = if channel == "alpha" {
                            alpha_rows(
                                &records["runs"],
                                &records["branches"],
                                &identity,
                                &this.inner.profile,
                            )
                        } else {
                            release_rows(
                                &records,
                                &channel,
                                text(&identity["version"]),
                                &this.inner.profile,
                            )
                        };
                        s.offer = s
                            .rows
                            .iter()
                            .find(|v| v["action"] == "Update" && v["id"] != s.dismissed)
                            .cloned()
                            .unwrap_or(Value::Null);
                        if !quiet {
                            s.status = if s.rows.is_empty() {
                                if channel == "alpha" {
                                    "No recent branch builds. Artifacts expire after 30 days."
                                } else {
                                    "No installable releases are available on this channel yet."
                                }
                            } else if channel == "alpha" {
                                "Choose a build to install."
                            } else {
                                "Release list is up to date."
                            }
                            .into();
                        }
                    }
                    Err(error) => {
                        s.preferences["checked_at"] = previous;
                        if !quiet {
                            s.status = error;
                        }
                    }
                }
            }
            s.checking = false;
            let pending = s.pending;
            s.pending = false;
            this.save(&mut s);
            drop(s);
            if pending {
                this.check(false, false);
            }
        });
    }
    fn load_notes(&self) {
        let previous = text(&self.inner.state.lock().unwrap().preferences["last_seen"]).to_owned();
        let current = text(&self.inner.identity.lock().unwrap()["version"]).to_owned();
        if previous == current || previous.is_empty() {
            return;
        }
        let Ok(records) = self.records(false, false) else {
            return;
        };
        let sections = notes_since(&records, &previous, &current);
        let mut s = self.inner.state.lock().unwrap();
        if sections.is_empty() {
            s.preferences["last_seen"] = json!(current);
            self.save(&mut s);
            return;
        }
        let count = sections
            .iter()
            .flat_map(|v| text(&v["notes"]).lines())
            .filter(|line| line.starts_with("- ") || line.starts_with("* "))
            .count();
        s.notes = json!({"title":if sections.len()==1 {format!("What's new in {current}")} else {format!("What's new since {previous}")},"count":count,"sections":sections});
    }
    pub fn row(&self, id: &str) -> Result<Value, String> {
        let s = self.inner.state.lock().unwrap();
        s.rows
            .iter()
            .chain(std::iter::once(&s.offer))
            .find(|v| v["id"] == id)
            .cloned()
            .ok_or("This build is no longer listed. Check for updates again.".into())
    }
    fn install(&self, id: &str, busy: bool) -> Result<(), String> {
        let row = self.row(id)?;
        let cached = {
            let mut s = self.inner.state.lock().unwrap();
            if s.download["active"] == true {
                return Ok(());
            }
            if row["action"] == "Downgrade" && s.armed != id {
                s.armed = json!(id);
                s.status=format!("Install {}? Application files move backward. Your files and settings stay. Press Downgrade again to continue.",text(&row["version"]));
                return Ok(());
            }
            s.armed = Value::Null;
            if busy {
                s.status = "Stop compression before installing an update.".into();
                return Ok(());
            }
            if !cfg!(windows) {
                s.status = "Automatic installation is available for Windows builds. Open the release page for other systems.".into();
                return Ok(());
            }
            s.download = json!({"active":true,"percent":0,"status":"Preparing download"});
            s.downloaded.get(id).filter(|p| p.is_file()).cloned()
        };
        self.inner.cancel.store(false, Ordering::Relaxed);
        let this = self.clone();
        let id = id.to_owned();
        thread::spawn(move || {
            let result = if let Some(path) = cached {
                Ok(path)
            } else {
                this.inner.source.lock().unwrap().download(
                    &row,
                    &this.inner.folder.join("updates"),
                    &this.inner.cancel,
                    |progress| {
                        let mut s = this.inner.state.lock().unwrap();
                        if let Some(map) = progress.as_object() {
                            for (key, value) in map {
                                s.download[key] = value.clone();
                            }
                        }
                    },
                )
            };
            let mut s = this.inner.state.lock().unwrap();
            match result {
                Ok(path) => {
                    s.downloaded.insert(id, path.clone());
                    s.ready = Some(path);
                    s.download["status"] = json!("Preparing installer");
                }
                Err(error) => {
                    s.status = error;
                    s.download["active"] = json!(false);
                }
            }
        });
        Ok(())
    }
    pub fn poll_ready_install(&self) -> Option<PathBuf> {
        self.inner.state.lock().unwrap().ready.take()
    }
    pub fn defer_install(&self) {
        let mut s = self.inner.state.lock().unwrap();
        s.download["active"] = json!(false);
        s.status="Compression started during the download. The installer is saved. Stop compression and press the install action again.".into();
    }
    pub fn launch_ready_install(&self, path: &Path) -> Result<bool, String> {
        let result = if self.inner.closed.load(Ordering::Relaxed)
            || self.inner.cancel.load(Ordering::Relaxed)
        {
            Err("Update cancelled.".into())
        } else {
            launch_installer(path, &self.inner.folder.join("updates"))
        };
        let mut s = self.inner.state.lock().unwrap();
        s.download["active"] = json!(false);
        s.status = match &result {
            Ok(true) => "Installer started. Restarting the app.".into(),
            Ok(false) => "The installer did not start. The current app remains open.".into(),
            Err(e) => e.clone(),
        };
        result
    }
    pub fn action(&self, method: &str, args: Value, busy: bool) -> Result<Value, String> {
        let arg = args.as_array().and_then(|v| v.first()).unwrap_or(&args);
        match method {
            "get_updates" => {}
            "check_updates" => self.check(false, true),
            "set_update_channel" => {
                {
                    let mut s = self.inner.state.lock().unwrap();
                    s.preferences["channel"] =
                        json!(if matches!(text(arg), "stable" | "prerelease" | "alpha") {
                            text(arg)
                        } else {
                            "stable"
                        });
                    s.offer = Value::Null;
                    s.armed = Value::Null;
                    s.rows.clear();
                    self.save(&mut s);
                }
                self.check(false, false);
            }
            "set_automatic_updates" => {
                let mut s = self.inner.state.lock().unwrap();
                s.preferences["automatic"] = json!(arg.as_bool().unwrap_or(true));
                s.armed = Value::Null;
                self.save(&mut s);
            }
            "disarm_downgrade" => self.inner.state.lock().unwrap().armed = Value::Null,
            "dismiss_update" => {
                let mut s = self.inner.state.lock().unwrap();
                s.dismissed = s.offer["id"].clone();
                s.offer = Value::Null;
                s.armed = Value::Null;
            }
            "install_update" => self.install(text(arg), busy)?,
            "dismiss_whats_new" | "close_whats_new" => {
                let mut s = self.inner.state.lock().unwrap();
                s.notes = Value::Null;
                s.preferences["last_seen"] = self.inner.identity.lock().unwrap()["version"].clone();
                self.save(&mut s);
            }
            "read_whats_new" => {
                let this = self.clone();
                thread::spawn(move || this.load_notes());
            }
            "open_update_page" => {
                let row = self.row(text(arg))?;
                let url = text(&row["url"]);
                if !safe_url(url) {
                    return Err("The update address is not trusted.".into());
                }
                self.inner.state.lock().unwrap().armed = Value::Null;
                return Ok(json!({"open_url":url}));
            }
            _ => return Err("Unknown update action.".into()),
        }
        Ok(self.snapshot())
    }
    pub fn close(&self) {
        self.inner.closed.store(true, Ordering::Relaxed);
        self.inner.cancel.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn settled(updater: &Updater) -> Value {
        let deadline = Instant::now() + Duration::from_secs(3);
        while updater.snapshot()["checking"] == true {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(5));
        }
        updater.snapshot()
    }
    fn profile() -> Value {
        json!({"owner":"mkiera","repository":"BlubberBound","display_name":"BlubberBound","installer_asset":"BlubberBound-Setup.exe","alpha_artifact":"BlubberBound-Setup","alpha_workflow":"build-test.yml"})
    }
    fn release(v: &str, id: u64) -> Value {
        json!({"id":id,"tag_name":format!("v{v}"),"body":"- Change.\r\n<!-- app-notes-end -->\nInstall","assets":[{"id":id,"name":"BlubberBound-Setup.exe","size":100,"browser_download_url":"https://github.com/file"}]})
    }
    #[test]
    fn stable_and_beta_ordering() {
        let records = json!([
            release("1.0.0", 1),
            release("1.2.0-beta.11", 2),
            release("1.2.0-beta.2", 3),
            release("1.1.0", 4)
        ]);
        let rows = release_rows(&records, "prerelease", "1.0.0", &profile());
        assert_eq!(rows[0]["version"], "1.2.0-beta.11");
        assert_eq!(rows[1]["version"], "1.2.0-beta.2");
        assert_eq!(
            release_rows(&records, "stable", "1.0.0", &profile()).len(),
            2
        );
    }
    #[test]
    fn version_metadata_does_not_change_direction() {
        let rows = release_rows(
            &json!([
                release("1.0.0", 1),
                release("1.1.0+hash", 2),
                release("1.2.0", 3)
            ]),
            "stable",
            "1.1.0",
            &profile(),
        );
        assert_eq!(
            rows.iter().map(|v| text(&v["action"])).collect::<Vec<_>>(),
            vec!["Update", "Reinstall", "Downgrade"]
        );
        assert_eq!(rows[1]["running"], true);
    }
    #[test]
    fn unsafe_addresses_and_names() {
        for url in [
            "http://github.com/f",
            "https://github.com.evil.example/f",
            "https://token@api.github.com/f",
            "https://github.com:99/f",
            "https://github.com/f\n",
            "https://github.com/f#fragment",
        ] {
            assert!(!safe_url(url), "{url}");
        }
        for name in [
            "../BlubberBound-Setup.exe",
            "a\\BlubberBound-Setup.exe",
            ".hidden-Setup.exe",
            "bad name-Setup.exe",
            "BlubberBound.exe",
        ] {
            assert!(!safe_installer(name));
        }
        assert!(safe_url(
            "https://release-assets.githubusercontent.com/f?signature=value"
        ));
        assert!(safe_installer("BlubberBound-Setup.exe"));
    }
    #[test]
    fn redirects_strip_cross_host_credentials_without_restoring_them() {
        let initial = reqwest::Url::parse("https://api.github.com/repos/test/assets/1").unwrap();
        let (same, auth) = redirect_target(&initial, "/other", true).unwrap();
        assert_eq!(same.as_str(), "https://api.github.com/other");
        assert!(auth);
        let (external, auth) = redirect_target(
            &same,
            "https://release-assets.githubusercontent.com/file",
            auth,
        )
        .unwrap();
        assert!(!auth);
        let (_, auth) = redirect_target(&external, "https://api.github.com/back", auth).unwrap();
        assert!(!auth);
        assert!(!redirect_target(&initial, "/other", false).unwrap().1);
    }
    #[test]
    fn redirect_rejects_untrusted_hosts_credentials_and_control_characters() {
        let initial = reqwest::Url::parse("https://api.github.com/file").unwrap();
        for location in [
            "https://evil.example/file",
            "http://github.com/file",
            "https://token@api.github.com/file",
            "https://api.github.com/file\n",
            "https://github.com:99/file",
            "https://github.com/file#fragment",
        ] {
            assert!(
                redirect_target(&initial, location, true).is_err(),
                "{location}"
            );
        }
    }
    #[test]
    fn stream_download_requires_exact_nonempty_size_and_removes_partial() {
        let folder = tempfile::tempdir().unwrap();
        let cancel = AtomicBool::new(false);
        for bytes in [b"".as_slice(), b"MZtes", b"MZtestX"] {
            let path = folder.path().join("download.part");
            assert!(copy_download(bytes, &path, Some(6), 100, &cancel, &|_| {}).is_err());
            assert!(!path.exists());
        }
        let path = folder.path().join("download.part");
        copy_download(b"MZtest".as_slice(), &path, Some(6), 100, &cancel, &|_| {}).unwrap();
        assert_eq!(fs::read(path).unwrap(), b"MZtest");
    }
    #[test]
    fn stream_ceiling_cancellation_and_read_failure_remove_partial() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("download.part");
        let cancel = AtomicBool::new(false);
        assert!(copy_download(b"MZtest".as_slice(), &path, None, 5, &cancel, &|_| {}).is_err());
        assert!(!path.exists());
        assert!(
            copy_download(b"MZtest".as_slice(), &path, Some(6), 100, &cancel, &|_| {
                cancel.store(true, Ordering::Relaxed)
            })
            .is_err()
        );
        assert!(!path.exists());
        cancel.store(false, Ordering::Relaxed);
        struct Broken;
        impl Read for Broken {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("disconnected"))
            }
        }
        assert!(copy_download(Broken, &path, None, 100, &cancel, &|_| {}).is_err());
        assert!(!path.exists());
    }
    #[test]
    fn stream_does_not_replace_or_remove_existing_path() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("download.part");
        fs::write(&path, b"existing").unwrap();
        assert!(copy_download(
            b"MZtest".as_slice(),
            &path,
            Some(6),
            100,
            &AtomicBool::new(false),
            &|_| {}
        )
        .is_err());
        assert_eq!(fs::read(path).unwrap(), b"existing");
    }
    #[test]
    fn invalid_download_selection_is_rejected_before_discovery() {
        let folder = tempfile::tempdir().unwrap();
        let mut source = Source::new(profile(), "1.0.0".into());
        for row in [
            json!({"kind":"other"}),
            json!({"kind":"alpha","run_id":0}),
            json!({"kind":"release","asset":{"name":"../bad-Setup.exe"}}),
        ] {
            assert!(source
                .download(&row, folder.path(), &AtomicBool::new(false), |_| {})
                .is_err());
            assert!(source.private.is_none());
            assert_eq!(fs::read_dir(folder.path()).unwrap().count(), 0);
        }
    }
    #[test]
    fn invalid_assets_are_filtered() {
        for (key, value) in [
            ("name", json!("Other-Setup.exe")),
            ("size", json!(true)),
            ("size", json!(0)),
            ("size", json!(MAX_INSTALLER + 1)),
            ("browser_download_url", json!("https://evil.example/f")),
        ] {
            let mut r = release("1.0.0", 1);
            r["assets"][0][key] = value;
            assert!(release_rows(&json!([r]), "stable", "0.9.0", &profile()).is_empty());
        }
    }
    #[test]
    fn release_notes_cut_installation_and_beta_duplicates() {
        let records = json!([
            release("1.0.0", 1),
            release("1.1.0-beta.1", 2),
            release("1.1.0", 3),
            release("1.2.0", 4)
        ]);
        let notes = notes_since(&records, "1.0.0", "1.2.0");
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[0]["notes"], "- Change.");
        assert!(notes_since(&records, "", "1.2.0").is_empty());
    }
    #[test]
    fn beta_notes_include_intermediate_stable_and_beta_versions() {
        let records = json!([
            release("1.0.0", 1),
            release("1.1.0-beta.1", 2),
            release("1.1.0", 3),
            release("1.2.0-beta.1", 4),
            release("1.2.0", 5)
        ]);
        let notes = notes_since(&records, "1.0.0", "1.2.0-beta.1");
        assert_eq!(
            notes
                .iter()
                .map(|r| text(&r["version"]))
                .collect::<Vec<_>>(),
            vec!["1.2.0-beta.1", "1.1.0", "1.1.0-beta.1"]
        );
        assert!(notes_since(&records, "1.2.0", "1.1.0").is_empty());
        assert!(notes_since(&records, "1.2.0", "1.2.0").is_empty());
    }
    #[test]
    fn stable_and_beta_share_cached_records_and_manual_check_reoffers() {
        let folder = tempfile::tempdir().unwrap();
        let updater = Updater::new(profile(), folder.path().to_owned(), "1.0.0".into());
        updater.inner.cache.lock().unwrap().insert(
            "releases".into(),
            (
                now(),
                json!([
                    release("1.0.0", 1),
                    release("1.1.0", 2),
                    release("1.2.0-beta.1", 3)
                ]),
            ),
        );
        updater
            .action("set_automatic_updates", json!([false]), false)
            .unwrap();
        updater.check(false, false);
        assert_eq!(settled(&updater)["rows"].as_array().unwrap().len(), 2);
        assert_eq!(updater.snapshot()["offer"]["version"], "1.1.0");
        updater.action("dismiss_update", json!([]), false).unwrap();
        updater
            .action("set_update_channel", json!(["prerelease"]), false)
            .unwrap();
        let state = settled(&updater);
        assert_eq!(state["rows"].as_array().unwrap().len(), 3);
        assert_eq!(state["offer"]["version"], "1.2.0-beta.1");
        assert!(updater.inner.source.lock().unwrap().private.is_none());
    }
    #[test]
    fn alpha_quiet_check_never_fetches_or_offers() {
        let folder = tempfile::tempdir().unwrap();
        let updater = Updater::new(profile(), folder.path().to_owned(), "1.0.0".into());
        updater.inner.state.lock().unwrap().preferences["channel"] = json!("alpha");
        updater.check(true, true);
        assert_eq!(updater.snapshot()["checking"], false);
        assert!(updater.snapshot()["offer"].is_null());
        assert!(updater.inner.source.lock().unwrap().private.is_none());
    }
    #[test]
    fn automatic_off_suppresses_quiet_checks() {
        let folder = tempfile::tempdir().unwrap();
        let updater = Updater::new(profile(), folder.path().to_owned(), "1.0.0".into());
        updater
            .action("set_automatic_updates", json!([false]), false)
            .unwrap();
        updater.check(true, true);
        assert_eq!(updater.snapshot()["checking"], false);
        assert_eq!(updater.snapshot()["checked_at"], 0);
    }
    #[test]
    fn skipped_notes_are_saved_only_after_dismissal() {
        let folder = tempfile::tempdir().unwrap();
        let updater = Updater::new(profile(), folder.path().to_owned(), "1.2.0".into());
        updater.inner.state.lock().unwrap().preferences["last_seen"] = json!("1.0.0");
        updater.inner.cache.lock().unwrap().insert(
            "releases".into(),
            (
                now(),
                json!([
                    release("1.1.0-beta.1", 1),
                    release("1.1.0", 2),
                    release("1.2.0", 3)
                ]),
            ),
        );
        updater.load_notes();
        assert_eq!(
            updater.snapshot()["whats_new"]["sections"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            updater.inner.state.lock().unwrap().preferences["last_seen"],
            "1.0.0"
        );
        updater
            .action("dismiss_whats_new", json!([]), false)
            .unwrap();
        let saved: Value =
            serde_json::from_slice(&fs::read(folder.path().join("updates.json")).unwrap()).unwrap();
        assert_eq!(saved["last_seen"], "1.2.0");
        assert!(updater.snapshot()["whats_new"].is_null());
    }
    #[test]
    fn alpha_selects_newest_live_branches_and_exact_run() {
        let runs = json!([{"id":10,"run_number":1,"head_branch":"beta","head_sha":"abcd","conclusion":"success"},{"id":11,"run_number":2,"head_branch":"Beta","head_sha":"abcd","conclusion":"success"},{"id":12,"run_number":3,"head_branch":"gone","conclusion":"success"}]);
        let rows = alpha_rows(
            &runs,
            &json!([{"name":"beta"}]),
            &json!({"run_id":"10","commit":"abcd"}),
            &profile(),
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["run_id"], 11);
        assert_eq!(rows[0]["running"], false);
    }
    #[test]
    fn preferences_preserve_unknown_fields() {
        let folder = tempfile::tempdir().unwrap();
        fs::write(
            folder.path().join("updates.json"),
            br#"{"future":{"keep":true},"channel":"bad","automatic":false,"checked_at":"bad"}"#,
        )
        .unwrap();
        let updater = Updater::new(profile(), folder.path().to_owned(), "1.0.0".into());
        assert_eq!(updater.snapshot()["channel"], "stable");
        assert_eq!(updater.snapshot()["automatic"], false);
        let saved: Value =
            serde_json::from_slice(&fs::read(folder.path().join("updates.json")).unwrap()).unwrap();
        assert_eq!(saved["future"]["keep"], true);
        assert_eq!(saved["last_seen"], "1.0.0");
    }
    #[test]
    fn downgrade_requires_second_press_and_busy_blocks_download() {
        let folder = tempfile::tempdir().unwrap();
        let updater = Updater::new(profile(), folder.path().to_owned(), "1.0.0".into());
        updater.inner.state.lock().unwrap().rows =
            release_rows(&json!([release("0.9.0", 1)]), "stable", "1.0.0", &profile());
        updater
            .action("install_update", json!(["release:1"]), true)
            .unwrap();
        assert_eq!(updater.snapshot()["armed_id"], "release:1");
        updater
            .action("install_update", json!(["release:1"]), true)
            .unwrap();
        assert_eq!(updater.snapshot()["download"]["active"], false);
        assert!(text(&updater.snapshot()["status"]).contains("Stop compression"));
    }
    #[cfg(windows)]
    #[test]
    fn second_downgrade_press_prepares_cached_installer_without_launching() {
        let folder = tempfile::tempdir().unwrap();
        let updater = Updater::new(profile(), folder.path().to_owned(), "1.0.0".into());
        let path = folder.path().join("old-Setup.exe");
        fs::write(&path, b"MZtest").unwrap();
        {
            let mut state = updater.inner.state.lock().unwrap();
            state.rows = release_rows(&json!([release("0.9.0", 1)]), "stable", "1.0.0", &profile());
            state.downloaded.insert("release:1".into(), path.clone());
        }
        updater
            .action("install_update", json!(["release:1"]), false)
            .unwrap();
        assert_eq!(updater.snapshot()["download"]["active"], false);
        assert!(updater.poll_ready_install().is_none());
        updater
            .action("install_update", json!(["release:1"]), false)
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        let ready = loop {
            if let Some(path) = updater.poll_ready_install() {
                break path;
            }
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(5));
        };
        assert_eq!(ready, path);
        assert_eq!(updater.snapshot()["download"]["active"], true);
        updater.defer_install();
        assert!(path.is_file());
    }
    #[test]
    fn installer_startup_requires_success_or_viable_log() {
        let folder = tempfile::tempdir().unwrap();
        let log = folder.path().join("installer.log");
        assert_eq!(
            wait_for_installer(|| Ok(Some(0)), &log, Duration::ZERO).unwrap(),
            true
        );
        assert!(wait_for_installer(|| Ok(Some(2)), &log, Duration::ZERO)
            .unwrap_err()
            .contains("code 2"));
        assert!(wait_for_installer(|| Ok(None), &log, Duration::ZERO)
            .unwrap_err()
            .contains("confirmed startup"));
        fs::write(&log, b"Starting installation").unwrap();
        assert!(wait_for_installer(|| Ok(None), &log, Duration::ZERO).unwrap());
        fs::write(&log, b"Message box: Close the application").unwrap();
        assert!(wait_for_installer(|| Ok(None), &log, Duration::ZERO)
            .unwrap_err()
            .contains("waiting"));
        fs::write(&log, b"").unwrap();
        assert!(startup_log(&log).is_err());
    }
    #[test]
    fn installer_logs_support_both_utf16_byte_orders_and_bound_size() {
        let folder = tempfile::tempdir().unwrap();
        let log = folder.path().join("installer.log");
        let content = "Message box: Continue?\nUser chose Yes\nInstalling";
        for little in [true, false] {
            let mut bytes = if little {
                vec![255, 254]
            } else {
                vec![254, 255]
            };
            for unit in content.encode_utf16() {
                bytes.extend(if little {
                    unit.to_le_bytes()
                } else {
                    unit.to_be_bytes()
                });
            }
            fs::write(&log, bytes).unwrap();
            assert!(startup_log(&log).unwrap());
        }
        fs::write(&log, vec![b'x'; 4 * 1024 * 1024 + 1]).unwrap();
        assert!(startup_log(&log)
            .unwrap_err()
            .contains("unexpectedly large"));
    }
    #[test]
    fn installer_path_requires_validated_file_in_updates_folder() {
        let folder = tempfile::tempdir().unwrap();
        let updates = folder.path().join("updates");
        fs::create_dir(&updates).unwrap();
        let outside = folder.path().join("test-Setup.exe");
        fs::write(&outside, b"MZtest").unwrap();
        assert!(installer_path(&outside, &updates).is_err());
        let inside = updates.join("test-Setup.exe");
        fs::write(&inside, b"MZtest").unwrap();
        assert!(installer_path(&inside, &updates).is_ok());
        let wrong = updates.join("test.exe");
        fs::write(&wrong, b"MZtest").unwrap();
        assert!(installer_path(&wrong, &updates).is_err());
    }
    #[test]
    fn invalid_installer_launch_clears_active_state_and_keeps_app_open() {
        let folder = tempfile::tempdir().unwrap();
        let updater = Updater::new(profile(), folder.path().to_owned(), "1.0.0".into());
        updater.inner.state.lock().unwrap().download["active"] = json!(true);
        assert!(updater
            .launch_ready_install(&folder.path().join("missing-Setup.exe"))
            .is_err());
        assert_eq!(updater.snapshot()["download"]["active"], false);
        assert!(!text(&updater.snapshot()["status"]).is_empty());
    }
    #[test]
    fn installer_signature_and_location_are_validated() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("test-Setup.exe");
        fs::write(&path, b"XXinvalid").unwrap();
        assert!(validate_binary(&path).is_err());
        fs::write(&path, b"MZtest").unwrap();
        assert!(validate_binary(&path).is_ok());
    }
    #[test]
    fn deferred_install_keeps_download_and_clears_active_state() {
        let folder = tempfile::tempdir().unwrap();
        let updater = Updater::new(profile(), folder.path().to_owned(), "1.0.0".into());
        let path = folder.path().join("release-1-Setup.exe");
        fs::write(&path, b"MZtest").unwrap();
        {
            let mut state = updater.inner.state.lock().unwrap();
            state.ready = Some(path.clone());
            state.downloaded.insert("release:1".into(), path.clone());
            state.download["active"] = json!(true);
        }
        assert_eq!(updater.poll_ready_install(), Some(path.clone()));
        assert_eq!(updater.poll_ready_install(), None);
        updater.defer_install();
        assert!(path.is_file());
        assert_eq!(updater.snapshot()["download"]["active"], false);
        assert!(text(&updater.snapshot()["status"]).contains("installer is saved"));
    }
    #[test]
    fn dismissing_offer_does_not_persist_suppression() {
        let folder = tempfile::tempdir().unwrap();
        let updater = Updater::new(profile(), folder.path().to_owned(), "1.0.0".into());
        updater.inner.state.lock().unwrap().offer = json!({"id":"release:2"});
        updater.action("dismiss_update", json!([]), false).unwrap();
        assert!(updater.snapshot()["offer"].is_null());
        assert_eq!(updater.inner.state.lock().unwrap().dismissed, "release:2");
        let saved: Value =
            serde_json::from_slice(&fs::read(folder.path().join("updates.json")).unwrap()).unwrap();
        assert!(saved.get("dismissed").is_none());
    }
    #[test]
    fn first_install_has_no_release_notes_or_requests() {
        let folder = tempfile::tempdir().unwrap();
        let updater = Updater::new(profile(), folder.path().to_owned(), "1.0.0".into());
        updater.load_notes();
        assert!(updater.snapshot()["whats_new"].is_null());
        assert!(updater.inner.cache.lock().unwrap().is_empty());
        assert!(updater.inner.source.lock().unwrap().private.is_none());
    }
    #[test]
    fn other_actions_disarm_downgrade() {
        let folder = tempfile::tempdir().unwrap();
        let updater = Updater::new(profile(), folder.path().to_owned(), "1.0.0".into());
        updater.inner.state.lock().unwrap().armed = json!("release:1");
        updater
            .action("set_automatic_updates", json!([false]), false)
            .unwrap();
        assert!(updater.snapshot()["armed_id"].is_null());
        assert_eq!(updater.snapshot()["automatic"], false);
    }
    #[test]
    fn prompt_detection_tracks_answers() {
        assert!(pending_prompt("Message box: Close the app"));
        assert!(!pending_prompt(
            "Message box: Close the app\nUser chose Yes\nInstalling"
        ));
        assert!(!pending_prompt(
            "Message box: Question\nMessage box returned 1"
        ));
    }
    #[test]
    fn archive_only_extracts_fixed_root_name() {
        let folder = tempfile::tempdir().unwrap();
        let archive = folder.path().join("input.zip");
        let mut writer = zip::ZipWriter::new(File::create(&archive).unwrap());
        for name in [
            "../BlubberBound-Setup.exe",
            "folder/BlubberBound-Setup.exe",
            "BlubberBound-Setup.exe",
        ] {
            writer
                .start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(b"MZtest").unwrap();
        }
        writer.finish().unwrap();
        let dest = folder.path().join("out.part");
        extract_installer(
            &archive,
            &dest,
            "BlubberBound-Setup.exe",
            &AtomicBool::new(false),
            &|_| {},
        )
        .unwrap();
        assert_eq!(fs::read(dest).unwrap(), b"MZtest");
    }
    #[test]
    fn archive_cancellation_stops_extraction() {
        let folder = tempfile::tempdir().unwrap();
        let archive = folder.path().join("input.zip");
        let mut writer = zip::ZipWriter::new(File::create(&archive).unwrap());
        writer
            .start_file(
                "BlubberBound-Setup.exe",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        writer.write_all(b"MZtest").unwrap();
        writer.finish().unwrap();
        assert!(extract_installer(
            &archive,
            &folder.path().join("out.part"),
            "BlubberBound-Setup.exe",
            &AtomicBool::new(true),
            &|_| {}
        )
        .is_err());
    }
}
