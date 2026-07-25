use super::{build_shell_command, format_command_output};

#[test]
fn format_command_output_truncates_on_utf8_boundary() {
    let input = format!("{}é", "a".repeat(29_999));
    let output = format_command_output(input, None);
    assert!(output.ends_with("\n... (output truncated)"));
    assert!(output.starts_with(&"a".repeat(29_999)));
}

#[cfg(windows)]
#[tokio::test]
async fn build_shell_command_uses_cmd_and_executes_command() {
    let output = build_shell_command("echo hello-from-cmd")
        .output()
        .await
        .expect("run cmd command");
    assert!(output.status.success(), "cmd command should succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.to_ascii_lowercase().contains("hello-from-cmd"),
        "unexpected stdout: {}",
        stdout
    );
}

#[cfg(unix)]
#[tokio::test]
async fn build_shell_command_uses_disk_backed_scratch_directory() {
    let expected = super::tool_scratch_dir().expect("jcode scratch directory");
    let output = build_shell_command("printf '%s\\n%s\\n' \"$TMPDIR\" \"$JCODE_SCRATCH_DIR\"")
        .output()
        .await
        .expect("run bash command");
    assert!(output.status.success(), "bash command should succeed");
    let stdout = String::from_utf8(output.stdout).expect("utf-8 scratch paths");
    let paths = stdout.lines().collect::<Vec<_>>();
    let expected = expected.to_string_lossy().into_owned();
    assert_eq!(paths, vec![expected.as_str(), expected.as_str()]);
    assert!(std::path::Path::new(&expected).is_dir());
}
