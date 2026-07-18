//! Snapshot guard for the OpenAPI surface.
//!
//! The spec covers every HTTP route exposed by `koharu-rpc::api::api()`. A
//! change to any handler's path, body, or response will churn this file.
//! Regenerate with `cargo insta review` after intended changes.

#[test]
fn openapi_paths_snapshot() {
    let (_, spec) = koharu_rpc::api::api();

    let json = serde_json::to_value(&spec).expect("serialize OpenAPI");
    let mut paths: Vec<(String, Vec<String>)> = json["paths"]
        .as_object()
        .expect("paths object")
        .iter()
        .map(|(path, item)| {
            let mut methods: Vec<String> = item
                .as_object()
                .map(|o| {
                    o.keys()
                        .filter(|k| {
                            matches!(k.as_str(), "get" | "post" | "put" | "patch" | "delete")
                        })
                        .cloned()
                        .collect()
                })
                .unwrap_or_default();
            methods.sort();
            (path.clone(), methods)
        })
        .collect();
    paths.sort();

    insta::assert_debug_snapshot!(paths);
}

#[test]
fn source_text_policy_only_extends_pipeline_config() {
    let (_, spec) = koharu_rpc::api::api();
    let json = serde_json::to_value(&spec).expect("serialize OpenAPI");
    let schemas = json["components"]["schemas"]
        .as_object()
        .expect("schemas object");

    let pipeline = schemas["PipelineConfig"]["properties"]
        .as_object()
        .expect("PipelineConfig properties");
    assert!(pipeline.contains_key("source_text_policy"));

    let request = schemas["StartPipelineRequest"]["properties"]
        .as_object()
        .expect("StartPipelineRequest properties");
    assert!(!request.contains_key("sourceTextPolicy"));
}

#[test]
fn typography_contracts_extend_config_progress_and_text_without_changing_start_request() {
    let (_, spec) = koharu_rpc::api::api();
    let json = serde_json::to_value(&spec).expect("serialize OpenAPI");
    let schemas = json["components"]["schemas"]
        .as_object()
        .expect("schemas object");

    let app_config = schemas["AppConfig"]["properties"]
        .as_object()
        .expect("AppConfig properties");
    assert!(app_config.contains_key("typography_planner"));

    let config_patch = schemas["ConfigPatch"]["properties"]
        .as_object()
        .expect("ConfigPatch properties");
    assert!(config_patch.contains_key("typographyPlanner"));

    let pipeline = schemas["PipelineConfig"]["properties"]
        .as_object()
        .expect("PipelineConfig properties");
    assert!(pipeline.contains_key("typography_planner"));

    let request = schemas["StartPipelineRequest"]["properties"]
        .as_object()
        .expect("StartPipelineRequest properties");
    assert!(!request.contains_key("typography"));
    assert!(!request.contains_key("typographyPlanner"));

    let steps = schemas["PipelineStep"]["enum"]
        .as_array()
        .expect("PipelineStep enum");
    assert!(steps.iter().any(|step| step == "typography"));

    for schema in ["TextData", "TextDataPatch"] {
        let properties = schemas[schema]["properties"]
            .as_object()
            .expect("text properties");
        assert!(properties.contains_key("typographyPlanVerified"));
    }
}

#[test]
fn custom_image_layer_route_is_not_exposed() {
    let (_, spec) = koharu_rpc::api::api();
    let json = serde_json::to_value(&spec).expect("serialize OpenAPI");
    let paths = json["paths"].as_object().expect("paths object");

    assert!(!paths.contains_key("/pages/{id}/image-layers"));
}
