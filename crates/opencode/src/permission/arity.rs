//! Shell command splitting for fine-grained bash permission checks.
//!
//! When a user runs `git push && rm -rf /`, the naive permission check
//! against the full string would either allow the whole thing or deny
//! the whole thing. That's wrong: the typical desired policy is "allow
//! safe git commands, deny rm -rf". We split the command on shell
//! operators (`&&`, `||`, `;`, `|`) and check each segment
//! independently against the rule set. If any segment is denied, the
//! whole command is denied.
//!
//! This is the Rust port of the role `tool/shell.ts` plays in TS
//! opencode: extracting per-sub-command boundaries before applying
//! permission rules. The TS version uses tree-sitter-bash for a full
//! AST; we use a quote-aware character scanner which catches the same
//! everyday cases (`&&`, `;`, pipes) without pulling in a 1MB+ WASM
//! grammar. Trade-off: we don't handle here-docs, command substitution
//! `$(...)`, or backticks. Users with extreme inputs should also rely
//! on the deny-by-default permission ruleset.

/// Split `command` into top-level segments separated by shell operators.
/// Quoted regions (single- or double-quoted) are preserved as-is, so
/// `echo "a; b"` is one segment.
///
/// Empty / whitespace-only segments (which would arise from things like
/// trailing `;`) are dropped.
pub fn split_commands(command: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut chars = command.chars().peekable();
    let mut in_single = false;
    let mut in_double = false;
    let mut escape = false;

    while let Some(c) = chars.next() {
        if escape {
            current.push(c);
            escape = false;
            continue;
        }

        match c {
            '\\' if !in_single => {
                current.push(c);
                escape = true;
            }
            '\'' if !in_double => {
                in_single = !in_single;
                current.push(c);
            }
            '"' if !in_single => {
                in_double = !in_double;
                current.push(c);
            }
            '&' | '|' if !in_single && !in_double && chars.peek() == Some(&c) => {
                // `&&` or `||`
                chars.next();
                push_trimmed(&mut segments, &mut current);
            }
            ';' | '|' if !in_single && !in_double => {
                push_trimmed(&mut segments, &mut current);
            }
            _ => current.push(c),
        }
    }
    push_trimmed(&mut segments, &mut current);
    segments
}

fn push_trimmed(segments: &mut Vec<String>, current: &mut String) {
    let s = current.trim().to_string();
    if !s.is_empty() {
        segments.push(s);
    }
    current.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_command_is_one_segment() {
        assert_eq!(split_commands("ls -la"), vec!["ls -la"]);
    }

    #[test]
    fn double_ampersand_splits() {
        assert_eq!(
            split_commands("git push && rm -rf /"),
            vec!["git push", "rm -rf /"]
        );
    }

    #[test]
    fn semicolon_splits() {
        assert_eq!(
            split_commands("echo a ; echo b"),
            vec!["echo a", "echo b"]
        );
    }

    #[test]
    fn pipe_splits() {
        assert_eq!(
            split_commands("ls | grep foo"),
            vec!["ls", "grep foo"]
        );
    }

    #[test]
    fn double_pipe_splits() {
        assert_eq!(
            split_commands("false || true"),
            vec!["false", "true"]
        );
    }

    #[test]
    fn operators_inside_quotes_do_not_split() {
        assert_eq!(
            split_commands(r#"echo "a; b && c""#),
            vec![r#"echo "a; b && c""#]
        );
        assert_eq!(
            split_commands("echo 'a | b'"),
            vec!["echo 'a | b'"]
        );
    }

    #[test]
    fn escaped_operator_does_not_split() {
        assert_eq!(
            split_commands(r"echo \&\& done"),
            vec![r"echo \&\& done"]
        );
    }

    #[test]
    fn empty_segments_dropped() {
        assert_eq!(split_commands(";; ls ;"), vec!["ls"]);
        assert_eq!(split_commands(""), Vec::<String>::new());
    }

    #[test]
    fn mixed_operators() {
        assert_eq!(
            split_commands("a && b ; c | d || e"),
            vec!["a", "b", "c", "d", "e"]
        );
    }
}
