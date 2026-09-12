fn main() {
    println!("cargo:rerun-if-changed=../build_info.json");
    println!("cargo:rerun-if-changed=../version.txt");
    println!("cargo:rerun-if-changed=../app_profile.json");
    let identity: serde_json::Value = std::fs::read_to_string("../build_info.json")
        .ok()
        .map(|text| serde_json::from_str(&text).expect("Invalid build_info.json"))
        .unwrap_or_else(|| {
            serde_json::json!({
                "version": std::fs::read_to_string("../version.txt").expect("Missing version.txt").trim(),
                "commit": "", "branch": "", "run_id": ""
            })
        });
    let version = identity["version"].as_str().expect("Missing build version");
    println!("cargo:rustc-env=SQUEEZE_BUILD_VERSION={version}");
    println!("cargo:rustc-env=SQUEEZE_BUILD_IDENTITY_JSON={identity}");
    tauri_build::build()
}
