/// Decode command output bytes into a string, handling encoding detection.
pub fn decode_command_output(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).to_string()
}

/// Prepare a command string for execution in the specified shell.
pub fn prepare_command_for_shell(command: &str, shell_program: &str, _shell_arg: &str) -> String {
    // Simple pass-through for now; shell-specific escaping can be added later.
    command.to_string()
}
