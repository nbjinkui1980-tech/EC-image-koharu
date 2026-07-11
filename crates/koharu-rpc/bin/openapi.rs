//! Emit the OpenAPI spec for the current router to a JSON file.

use std::fs;

fn main() {
    let (_, spec) = koharu_rpc::api::api();
    let json = normalize_platform_defaults(
        spec.to_pretty_json().unwrap(),
        koharu_runtime::default_app_data_root().as_str(),
    );
    let path = std::env::args()
        .nth(1)
        .expect("Output path for OpenAPI spec JSON must be provided as the first argument");
    fs::write(path, json).unwrap();
}

fn normalize_platform_defaults(spec: String, app_data_path: &str) -> String {
    let platform_path = serde_json::to_string(app_data_path).unwrap();
    let portable_path = serde_json::to_string("${APP_DATA}/Koharu").unwrap();
    let needle = format!("\"path\": {platform_path}");
    assert!(spec.contains(&needle), "AppConfig default path is missing");
    spec.replacen(&needle, &format!("\"path\": {portable_path}"), 1)
}

#[cfg(test)]
mod tests {
    #[test]
    fn app_data_default_is_platform_neutral() {
        let (_, spec) = koharu_rpc::api::api();
        let platform_path = koharu_runtime::default_app_data_root();
        let spec = super::normalize_platform_defaults(
            spec.to_pretty_json().unwrap(),
            platform_path.as_str(),
        );
        let spec: serde_json::Value = serde_json::from_str(&spec).unwrap();

        assert_eq!(
            spec.pointer("/components/schemas/AppConfig/properties/data/default/path"),
            Some(&serde_json::Value::String("${APP_DATA}/Koharu".to_string()))
        );
    }
}
