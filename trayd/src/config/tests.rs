use super::default_socket_path;

#[test]
fn default_socket_path_uses_xdg_runtime_dir() {
    // Safe to mutate env in tests because we restore it afterwards.
    let key = "XDG_RUNTIME_DIR";
    let saved = std::env::var(key).ok();

    // SAFETY: single-threaded test; no other threads read this variable.
    unsafe { std::env::set_var(key, "/run/user/1000") };
    let path = default_socket_path().expect("should succeed with XDG_RUNTIME_DIR set");
    assert_eq!(path.to_str().unwrap(), "/run/user/1000/trayd.sock");

    // Restore.
    match saved {
        // SAFETY: same as above.
        Some(v) => unsafe { std::env::set_var(key, v) },
        None => unsafe { std::env::remove_var(key) },
    }
}

#[test]
fn default_socket_path_fails_without_xdg_runtime_dir() {
    let key = "XDG_RUNTIME_DIR";
    let saved = std::env::var(key).ok();

    // SAFETY: single-threaded test.
    unsafe { std::env::remove_var(key) };
    assert!(default_socket_path().is_err());

    if let Some(v) = saved {
        // SAFETY: same as above.
        unsafe { std::env::set_var(key, v) };
    }
}
