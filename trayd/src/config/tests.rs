use super::Config;

#[test]
fn defaults_are_sensible() {
    let cfg = Config::default();
    assert!(cfg.socket_path.file_name().unwrap() == "trayd.sock");
    assert!(!cfg.log_filter.is_empty());
}

#[test]
fn load_from_toml_string() {
    let toml = r#"
socket_path = "/tmp/test.sock"
log_filter  = "debug"
"#;
    let cfg: Config = toml::from_str(toml).unwrap();
    assert_eq!(cfg.socket_path.to_str().unwrap(), "/tmp/test.sock");
    assert_eq!(cfg.log_filter, "debug");
}

#[test]
fn missing_fields_fall_back_to_defaults() {
    let cfg: Config = toml::from_str("").unwrap();
    assert!(cfg.socket_path.ends_with("trayd.sock"));
    assert!(!cfg.log_filter.is_empty());
}

#[test]
fn load_returns_defaults_when_no_file() {
    let cfg = Config::load(None).unwrap();
    assert!(cfg.socket_path.ends_with("trayd.sock"));
}

#[test]
fn load_explicit_missing_path_errors() {
    let result = Config::load(Some(std::path::Path::new("/nonexistent/path/trayd.toml")));
    assert!(result.is_err());
}
