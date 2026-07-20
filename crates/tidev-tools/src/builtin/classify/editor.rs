use super::*;

/// Classify sed/perl/awk — only blocking if in-place flag is present.
pub(super) fn classify_editor(args: &[&str], _in_place_flags: &[&str]) -> Safety {
    let has_in_place = args.iter().any(|a| {
        // Exact match: -i
        if a == &"-i" {
            return true;
        }
        // Combined flags: single-dash flags containing 'i' (e.g. -pi, -i.bak)
        if let Some(flags) = a.strip_prefix('-') {
            // Only check single-dash flags, not --long-options
            if !flags.starts_with('-') {
                // Split off the value suffix (e.g. .bak in -i.bak)
                let flag_letters = flags.split('.').next().unwrap_or(flags);
                return flag_letters.contains('i');
            }
        }
        false
    });

    if has_in_place {
        Safety::WriteOperation
    } else {
        // Without -i, sed/perl output to stdout — read-only
        Safety::ReadOnly
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_i_flag_detected() {
        assert_eq!(
            classify_editor(&["-i", "file"], &["-i"]),
            Safety::WriteOperation
        );
    }

    #[test]
    fn combined_flag_with_i_detected() {
        assert_eq!(
            classify_editor(&["-pi", "-e", "s/foo/bar/"], &["-i"]),
            Safety::WriteOperation
        );
    }

    #[test]
    fn i_with_suffix_detected() {
        assert_eq!(
            classify_editor(&["-i.bak", "file"], &["-i"]),
            Safety::WriteOperation
        );
    }

    #[test]
    fn no_i_flag_is_read_only() {
        assert_eq!(
            classify_editor(&["-n", "-e", "s/foo/bar/", "file"], &["-i"]),
            Safety::ReadOnly
        );
    }

    #[test]
    fn long_option_not_mistaken_for_i() {
        // `--include` should not be treated as -i
        assert_eq!(classify_editor(&["--include"], &["-i"]), Safety::ReadOnly);
    }

    #[test]
    fn empty_args_is_read_only() {
        assert_eq!(classify_editor(&[], &["-i"]), Safety::ReadOnly);
    }

    #[test]
    fn unrelated_flags_are_read_only() {
        assert_eq!(
            classify_editor(&["-e", "s/foo/bar/", "-n"], &["-i"]),
            Safety::ReadOnly
        );
    }
}
