#![windows_subsystem = "windows"]

use std::{
    env, fs, thread,
    time::{Duration, Instant},
};

fn main() {
    let exe = env::current_exe().unwrap();
    let payload = exe.parent().unwrap();
    if let Some(log) = env::args().find_map(|arg| arg.strip_prefix("/LOG=").map(String::from)) {
        fs::write(log, "Message box: waiting for application exit").unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !payload.join("release-installer.txt").exists() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }
        fs::write(payload.join("installer-exited.txt"), "done").unwrap();
        return;
    }
    let install = payload.parent().unwrap();
    let version = fs::read_to_string(payload.join("version.txt")).unwrap();
    if version.trim() == "old" || version.trim() == "blocked" {
        env::set_current_dir(payload).unwrap();
        fs::write(install.join("started.txt"), "running").unwrap();
        let deadline = Instant::now() + Duration::from_secs(45);
        if version.trim() == "blocked" {
            while Instant::now() < deadline {
                thread::sleep(Duration::from_millis(20));
            }
            return;
        }
        while !install.join("update.log").exists() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }
        thread::sleep(Duration::from_millis(1500));
    } else {
        fs::write(install.join("relaunched.txt"), version).unwrap();
    }
}
