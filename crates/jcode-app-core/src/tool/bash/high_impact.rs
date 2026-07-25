//! Permissive visibility for unusually high-impact shell commands.
//!
//! Jcode intentionally does not put an approval gate in this path. The product
//! intent in `FEEDBACK.md` is maximum practical agent access with clear UI
//! feedback, so detection only publishes an informational activity and audit
//! log immediately before the command proceeds.

use crate::bus::{Bus, BusEvent, UiActivity};
use crate::tool::ToolContext;

const MAX_COMMAND_PREVIEW_CHARS: usize = 1200;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct HighImpactCommand {
    pub category: &'static str,
    pub reason: &'static str,
}

pub(super) fn publish_notice(
    command: &str,
    intent: Option<&str>,
    ctx: &ToolContext,
) -> Option<HighImpactCommand> {
    let impact = detect(command)?;
    let command_preview = truncate_chars(command.trim(), MAX_COMMAND_PREVIEW_CHARS);
    let intent_line = match intent.map(str::trim).filter(|intent| !intent.is_empty()) {
        Some(intent) => format!("\n- Intent: {intent}"),
        None => String::new(),
    };
    let message = format!(
        "⚠ **High-impact shell command running**\n\n\
         Jcode is proceeding immediately with full agent access. This notice is informational and did not pause or restrict execution.\n\n\
         - Impact: {}{}\n\n\
         ```sh\n{}\n```",
        impact.reason, intent_line, command_preview
    );
    let status_notice = format!("High-impact command running · {}", impact.reason);

    crate::logging::warn(&format!(
        "EVENT event=HIGH_IMPACT_COMMAND_EXECUTING access_policy=permissive session_id={} tool_call_id={} category={} command={:?}",
        ctx.session_id, ctx.tool_call_id, impact.category, command_preview
    ));
    Bus::global().publish(BusEvent::UiActivity(UiActivity::command(
        Some(ctx.session_id.clone()),
        message,
        Some(status_notice),
    )));
    Some(impact)
}

pub(super) fn detect(command: &str) -> Option<HighImpactCommand> {
    detect_with_depth(command, 0)
}

fn detect_with_depth(command: &str, depth: usize) -> Option<HighImpactCommand> {
    if depth > 4 {
        return None;
    }
    shell_segments(command)
        .iter()
        .find_map(|segment| detect_segment(segment, depth))
}

fn detect_segment(tokens: &[String], depth: usize) -> Option<HighImpactCommand> {
    let command_index = tokens.iter().position(|token| !is_assignment(token))?;
    let executable = executable_name(&tokens[command_index]);
    let args = &tokens[command_index + 1..];

    match executable {
        "rm" if rm_is_recursive(args) && has_operand(args) => Some(HighImpactCommand {
            category: "recursive_deletion",
            reason: "recursive file deletion",
        }),
        "find" if args.iter().any(|arg| arg == "-delete") => Some(HighImpactCommand {
            category: "bulk_find_deletion",
            reason: "bulk deletion through `find -delete`",
        }),
        "find" if find_executes_recursive_rm(args) => Some(HighImpactCommand {
            category: "bulk_find_deletion",
            reason: "recursive deletion through `find -exec`",
        }),
        "git" => detect_git(args),
        "shred" if has_operand(args) => Some(HighImpactCommand {
            category: "secure_deletion",
            reason: "secure file shredding",
        }),
        "wipefs" if has_flag(args, "-a", "--all") => Some(HighImpactCommand {
            category: "filesystem_signature_wipe",
            reason: "filesystem signature removal",
        }),
        name if name == "mkfs" || name.starts_with("mkfs.") => Some(HighImpactCommand {
            category: "filesystem_format",
            reason: "filesystem formatting",
        }),
        "dd" if dd_has_output(args) => Some(HighImpactCommand {
            category: "raw_overwrite",
            reason: "raw file or device overwrite with `dd`",
        }),
        "truncate" if has_flag(args, "-s", "--size") || has_long_assignment(args, "--size=") => {
            Some(HighImpactCommand {
                category: "file_truncation",
                reason: "explicit file truncation",
            })
        }
        "rsync"
            if has_flag(args, "--delete", "--delete-before")
                || has_flag(args, "--delete-after", "--delete-during") =>
        {
            Some(HighImpactCommand {
                category: "synchronized_deletion",
                reason: "bulk destination deletion during synchronization",
            })
        }
        "bash" | "sh" | "zsh" | "dash" | "fish" => {
            shell_script_argument(args).and_then(|script| detect_with_depth(script, depth + 1))
        }
        "eval" => detect_with_depth(&args.join(" "), depth + 1),
        "sudo" | "doas" | "env" | "command" | "nohup" | "nice" | "timeout" | "xargs" => {
            detect_wrapped_command(args, depth + 1)
        }
        _ => None,
    }
}

fn detect_git(args: &[String]) -> Option<HighImpactCommand> {
    let subcommand_index = args
        .iter()
        .position(|arg| arg == "clean" || arg == "reset")?;
    let subcommand = args[subcommand_index].as_str();
    let sub_args = &args[subcommand_index + 1..];
    match subcommand {
        "clean" if git_clean_is_forced(sub_args) && !git_clean_is_dry_run(sub_args) => {
            Some(HighImpactCommand {
                category: "forced_git_cleanup",
                reason: "forced deletion of untracked Git files",
            })
        }
        "reset" if sub_args.iter().any(|arg| arg == "--hard") => Some(HighImpactCommand {
            category: "hard_git_reset",
            reason: "discarding uncommitted Git changes with `reset --hard`",
        }),
        _ => None,
    }
}

fn detect_wrapped_command(args: &[String], depth: usize) -> Option<HighImpactCommand> {
    for index in 0..args.len() {
        let candidate = executable_name(&args[index]);
        if is_detectable_executable(candidate) {
            return detect_segment(&args[index..], depth);
        }
    }
    None
}

fn is_detectable_executable(name: &str) -> bool {
    matches!(
        name,
        "rm" | "find"
            | "git"
            | "shred"
            | "wipefs"
            | "mkfs"
            | "dd"
            | "truncate"
            | "rsync"
            | "bash"
            | "sh"
            | "zsh"
            | "dash"
            | "fish"
            | "eval"
            | "sudo"
            | "doas"
            | "env"
            | "command"
            | "nohup"
            | "nice"
            | "timeout"
            | "xargs"
    ) || name.starts_with("mkfs.")
}

fn executable_name(token: &str) -> &str {
    token.rsplit(['/', '\\']).next().unwrap_or(token)
}

fn is_assignment(token: &str) -> bool {
    let Some((name, _)) = token.split_once('=') else {
        return false;
    };
    !name.is_empty()
        && name
            .chars()
            .all(|character| character == '_' || character.is_ascii_alphanumeric())
        && name
            .chars()
            .next()
            .is_some_and(|character| character == '_' || character.is_ascii_alphabetic())
}

fn rm_is_recursive(args: &[String]) -> bool {
    args.iter().any(|arg| {
        arg == "--recursive"
            || (arg.starts_with('-')
                && !arg.starts_with("--")
                && arg[1..].chars().any(|flag| flag == 'r' || flag == 'R'))
    })
}

fn has_operand(args: &[String]) -> bool {
    let mut options_ended = false;
    args.iter().any(|arg| {
        if options_ended {
            return true;
        }
        if arg == "--" {
            options_ended = true;
            return false;
        }
        !arg.starts_with('-')
    })
}

fn has_flag(args: &[String], short: &str, long: &str) -> bool {
    args.iter().any(|arg| arg == short || arg == long)
}

fn has_long_assignment(args: &[String], prefix: &str) -> bool {
    args.iter().any(|arg| arg.starts_with(prefix))
}

fn find_executes_recursive_rm(args: &[String]) -> bool {
    args.iter()
        .position(|arg| arg == "-exec" || arg == "-execdir")
        .and_then(|index| detect_segment(&args[index + 1..], 0))
        .is_some_and(|impact| impact.category == "recursive_deletion")
}

fn dd_has_output(args: &[String]) -> bool {
    args.iter().any(|arg| {
        arg.strip_prefix("of=")
            .is_some_and(|target| !target.is_empty() && target != "/dev/null")
    })
}

fn git_clean_is_forced(args: &[String]) -> bool {
    args.iter().any(|arg| {
        arg == "--force"
            || (arg.starts_with('-') && !arg.starts_with("--") && arg[1..].contains('f'))
    })
}

fn git_clean_is_dry_run(args: &[String]) -> bool {
    args.iter().any(|arg| {
        arg == "--dry-run"
            || (arg.starts_with('-') && !arg.starts_with("--") && arg[1..].contains('n'))
    })
}

fn shell_script_argument(args: &[String]) -> Option<&str> {
    args.windows(2)
        .find(|pair| {
            pair[0] == "-c"
                || (pair[0].starts_with('-')
                    && !pair[0].starts_with("--")
                    && pair[0][1..].contains('c'))
        })
        .map(|pair| pair[1].as_str())
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let preview: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{preview}\n# … command preview truncated")
    } else {
        preview
    }
}

fn shell_segments(command: &str) -> Vec<Vec<String>> {
    let mut segments = Vec::new();
    let mut segment = Vec::new();
    let mut word = String::new();
    let mut chars = command.chars().peekable();
    let mut quote = None;

    while let Some(character) = chars.next() {
        if let Some(active_quote) = quote {
            if character == active_quote {
                quote = None;
            } else if character == '\\' && active_quote == '"' {
                if let Some(escaped) = chars.next() {
                    word.push(escaped);
                }
            } else {
                word.push(character);
            }
            continue;
        }

        match character {
            '\'' | '"' => quote = Some(character),
            '\\' => {
                if let Some(escaped) = chars.next() {
                    word.push(escaped);
                }
            }
            '#' if word.is_empty() => {
                for rest in chars.by_ref() {
                    if rest == '\n' {
                        break;
                    }
                }
                finish_segment(&mut word, &mut segment, &mut segments);
            }
            '$' if chars.peek() == Some(&'(') => {
                chars.next();
                finish_segment(&mut word, &mut segment, &mut segments);
            }
            ';' | '|' | '&' | '(' | ')' | '`' | '\n' => {
                finish_segment(&mut word, &mut segment, &mut segments);
            }
            character if character.is_whitespace() => finish_word(&mut word, &mut segment),
            _ => word.push(character),
        }
    }
    finish_segment(&mut word, &mut segment, &mut segments);
    segments
}

fn finish_word(word: &mut String, segment: &mut Vec<String>) {
    if !word.is_empty() {
        segment.push(std::mem::take(word));
    }
}

fn finish_segment(word: &mut String, segment: &mut Vec<String>, segments: &mut Vec<Vec<String>>) {
    finish_word(word, segment);
    if !segment.is_empty() {
        segments.push(std::mem::take(segment));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn category(command: &str) -> Option<&'static str> {
        detect(command).map(|impact| impact.category)
    }

    #[test]
    fn recursive_deletion_is_visible_without_classifying_normal_removal() {
        assert_eq!(category("rm -rf -- ~/old"), Some("recursive_deletion"));
        assert_eq!(category("/bin/rm -R ./build"), Some("recursive_deletion"));
        assert_eq!(category("rm one-file.txt"), None);
        assert_eq!(category("echo rm -rf /"), None);
    }

    #[test]
    fn wrappers_and_nested_shells_are_inspected() {
        assert_eq!(
            category("sudo env FOO=1 rm -rf /srv/cache"),
            Some("recursive_deletion")
        );
        assert_eq!(
            category("bash -c 'git reset --hard HEAD'"),
            Some("hard_git_reset")
        );
        assert_eq!(
            category("printf ok && find . -delete"),
            Some("bulk_find_deletion")
        );
        assert_eq!(
            category("echo $(rm -rf ./generated)"),
            Some("recursive_deletion")
        );
    }

    #[test]
    fn git_dry_run_stays_quiet_but_forced_cleanup_is_visible() {
        assert_eq!(category("git clean -ndx"), None);
        assert_eq!(category("git clean -fdx"), Some("forced_git_cleanup"));
        assert_eq!(category("git reset --soft HEAD~1"), None);
        assert_eq!(category("git reset --hard HEAD~1"), Some("hard_git_reset"));
    }

    #[test]
    fn device_and_bulk_overwrite_commands_are_visible() {
        assert_eq!(category("mkfs.ext4 /dev/sdb1"), Some("filesystem_format"));
        assert_eq!(
            category("dd if=image.iso of=/dev/sdb bs=4M"),
            Some("raw_overwrite")
        );
        assert_eq!(category("dd if=/dev/zero of=/dev/null"), None);
        assert_eq!(
            category("rsync -a --delete source/ target/"),
            Some("synchronized_deletion")
        );
    }
}
