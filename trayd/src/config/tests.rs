use super::Config;

#[test]
fn defaults_are_sensible() {
    let cfg = Config::default();
    assert_eq!(cfg.socket_path, None);
    assert!(!cfg.log_filter.is_empty());
}

#[test]
fn load_from_toml_string() {
    let toml = r#"
socket_path = "/tmp/test.sock"
log_filter  = "debug"
"#;
    let cfg: Config = toml::from_str(toml).unwrap();
    assert_eq!(
        cfg.socket_path.as_deref(),
        Some(std::path::Path::new("/tmp/test.sock"))
    );
    assert_eq!(cfg.log_filter, "debug");
}

#[test]
fn missing_fields_fall_back_to_defaults() {
    let cfg: Config = toml::from_str("").unwrap();
    assert_eq!(cfg.socket_path, None);
    assert!(!cfg.log_filter.is_empty());
}

#[test]
fn socket_path_prefers_config_then_runtime_dir() {
    use super::resolve_socket_path;
    use std::path::Path;
    let configured = Path::new("/run/custom.sock");
    assert_eq!(
        resolve_socket_path(Some(configured), Some("/run/user/1".into())).unwrap(),
        configured
    );
    assert_eq!(
        resolve_socket_path(None, Some("/run/user/1".into())).unwrap(),
        Path::new("/run/user/1/trayd.sock")
    );
}

#[test]
fn socket_path_never_falls_back_to_tmp() {
    use super::resolve_socket_path;
    assert!(resolve_socket_path(None, None).is_err());
    assert!(resolve_socket_path(None, Some("".into())).is_err());
}

#[test]
fn load_explicit_missing_path_errors() {
    let result = Config::load(Some(std::path::Path::new("/nonexistent/path/trayd.toml")));
    assert!(result.is_err());
}

#[test]
fn socket_flag_overrides_the_config_file() {
    let cfg: Config = toml::from_str(r#"socket_path = "/from/config.sock""#).unwrap();
    let kept = Config { ..cfg }.with_socket_override(None);
    assert_eq!(
        kept.socket_path.as_deref(),
        Some(std::path::Path::new("/from/config.sock"))
    );
    let overridden = kept.with_socket_override(Some("/from/cli.sock".into()));
    assert_eq!(
        overridden.socket_path.as_deref(),
        Some(std::path::Path::new("/from/cli.sock"))
    );
}
