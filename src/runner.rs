//! # command
//!
//! Runs task commands/scripts.
//!

#[cfg(test)]
#[path = "./runner_test.rs"]
mod runner_test;

use crate::types::{ErrorInfo, ScriptError, ScriptOptions, IoOptions};
use rand::distributions::Alphanumeric;
use rand::{thread_rng, Rng};
use std::env;
use std::env::current_dir;
use std::fs::{create_dir_all, remove_file, File};
use std::io::prelude::*;
use std::io::Error;
use std::iter;
use std::process::{Child, Command, ExitStatus, Stdio};

#[cfg(not(windows))]
use users::get_current_username;

/// Returns the exit code
fn get_exit_code(code: ExitStatus) -> i32 {
    if !code.success() {
        match code.code() {
            Some(value) => value,
            None => -1,
        }
    } else {
        0
    }
}

/// Creates a command builder for the given input.
fn create_command_builder(
    command_string: &str,
    args: &Vec<String>,
    options: &ScriptOptions,
) -> Command {
    let mut command = Command::new(&command_string);

    for arg in args.iter() {
        command.arg(arg);
    }

    match options.capture_input {
        IoOptions::Null => command.stdin(Stdio::null()),
        IoOptions::Inherit => command.stdin(Stdio::inherit()),
        IoOptions::Pipe => command.stdin(Stdio::piped()),
    };

    match options.capture_output {
        IoOptions::Null => command.stdout(Stdio::null()).stderr(Stdio::null()),
        IoOptions::Inherit => command.stdout(Stdio::inherit()).stderr(Stdio::inherit()),
        IoOptions::Pipe => command.stdout(Stdio::piped()).stderr(Stdio::piped()),
    };

    command
}

fn delete_file(file: &str) {
    remove_file(file).unwrap_or(());
}

#[cfg(windows)]
fn get_additional_temp_path() -> Option<String> {
    None
}

#[cfg(not(windows))]
fn get_additional_temp_path() -> Option<String> {
    let username = get_current_username();

    match username {
        Some(os_value) => match os_value.into_string() {
            Ok(value) => Some(value),
            Err(_) => None,
        },
        None => None,
    }
}

fn create_script_file(script: &String) -> Result<String, Error> {
    let name = env!("CARGO_PKG_NAME");

    let mut rng = thread_rng();
    let file_name: String = iter::repeat(())
        .map(|()| rng.sample(Alphanumeric))
        .take(10)
        .collect();

    let mut file_path = env::temp_dir();

    match get_additional_temp_path() {
        Some(additional_path) => file_path.push(additional_path),
        None => {}
    };

    file_path.push(name);

    // create parent directory
    match create_dir_all(&file_path) {
        Ok(_) => {
            file_path.push(file_name);
            if cfg!(windows) {
                file_path.set_extension("bat");
            } else {
                file_path.set_extension("sh");
            };

            let file_path_str = &file_path.to_str().unwrap_or("");

            match File::create(&file_path) {
                Ok(mut file) => match file.write_all(script.as_bytes()) {
                    Ok(_) => Ok(file_path_str.to_string()),
                    Err(error) => {
                        delete_file(&file_path_str);

                        Err(error)
                    }
                },
                Err(error) => Err(error),
            }
        }
        Err(error) => Err(error),
    }
}

fn modify_script(script: &String, options: &ScriptOptions) -> Result<String, ScriptError> {
    match current_dir() {
        Ok(cwd_holder) => {
            match cwd_holder.to_str() {
                Some(cwd) => {
                    // create cd command
                    let mut cd_command = "cd ".to_string();
                    cd_command.push_str(cwd);

                    let mut script_lines: Vec<String> = script
                        .trim()
                        .split("\n")
                        .map(|string| string.to_string())
                        .collect();

                    // check if first line is shebang line
                    let mut insert_index =
                        if script_lines.len() > 0 && script_lines[0].starts_with("#!") {
                            1
                        } else {
                            0
                        };

                    if !cfg!(windows) {
                        if options.exit_on_error {
                            script_lines.insert(insert_index, "set -e".to_string());
                            insert_index = insert_index + 1;
                        }

                        if options.print_commands {
                            script_lines.insert(insert_index, "set -x".to_string());
                            insert_index = insert_index + 1;
                        }
                    }

                    script_lines.insert(insert_index, cd_command);

                    script_lines.push("\n".to_string());

                    let updated_script = script_lines.join("\n");

                    Ok(updated_script)
                }
                None => Err(ScriptError {
                    info: ErrorInfo::Description(
                        "Unable to extract current working directory path.",
                    ),
                }),
            }
        }
        Err(error) => Err(ScriptError {
            info: ErrorInfo::IOError(error),
        }),
    }
}

/// Invokes the provided script content and returns a process handle.
fn spawn_script(
    script: &str,
    args: &Vec<String>,
    options: &ScriptOptions,
) -> Result<(Child, String), ScriptError> {
    match modify_script(&script.to_string(), &options) {
        Ok(updated_script) => match create_script_file(&updated_script) {
            Ok(file) => {
                let command = match options.runner {
                    Some(ref value) => value,
                    None => {
                        if cfg!(windows) {
                            "cmd.exe"
                        } else {
                            "sh"
                        }
                    }
                };

                let mut all_args = if cfg!(windows) {
                    vec!["/C".to_string(), file.to_string()]
                } else {
                    vec![file.to_string()]
                };

                all_args.extend(args.iter().cloned());

                let mut command = create_command_builder(&command, &all_args, &options);

                let result = command.spawn();

                match result {
                    Ok(child) => Ok((child, file.clone())),
                    Err(error) => {
                        delete_file(&file);

                        Err(ScriptError {
                            info: ErrorInfo::IOError(error),
                        })
                    }
                }
            }
            Err(error) => Err(ScriptError {
                info: ErrorInfo::IOError(error),
            }),
        },
        Err(error) => Err(error),
    }
}

/// Invokes the provided script content and returns a process handle.
///
/// # Arguments
///
/// * `script` - The script content
/// * `args` - The script command line arguments
/// * `options` - Options provided to the script runner
pub(crate) fn spawn(
    script: &str,
    args: &Vec<String>,
    options: &ScriptOptions,
) -> Result<Child, ScriptError> {
    let result = spawn_script(script, &args, &options);

    match result {
        Ok((child, _)) => Ok(child),
        Err(error) => Err(error),
    }
}

/// Invokes the provided script content and returns the invocation output.
///
/// # Arguments
///
/// * `script` - The script content
/// * `args` - The script command line arguments
/// * `options` - Options provided to the script runner
pub(crate) fn run(
    script: &str,
    args: &Vec<String>,
    options: &ScriptOptions,
) -> Result<(i32, String, String), ScriptError> {
    let result = spawn_script(script, &args, &options);

    match result {
        Ok((child, file)) => {
            let process_result = child.wait_with_output();

            delete_file(&file);

            match process_result {
                Ok(output) => {
                    let exit_code = get_exit_code(output.status);
                    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
                    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

                    Ok((exit_code, stdout, stderr))
                }
                Err(error) => Err(ScriptError {
                    info: ErrorInfo::IOError(error),
                }),
            }
        }
        Err(error) => Err(error),
    }
}
