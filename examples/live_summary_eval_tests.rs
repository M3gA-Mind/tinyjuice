use super::*;

#[test]
fn curl_config_keeps_authorization_out_of_arguments_and_removes_it_on_drop() {
    let config = write_curl_config("fixture-key").unwrap();
    let path = config.path().to_owned();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "header = \"Authorization: Bearer fixture-key\"\n"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    }
    drop(config);
    assert!(!path.exists());
}
