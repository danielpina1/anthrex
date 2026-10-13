//! Milestone 9.15 decision 11's marker breaking. Task M9.15.6 adds [`break_markers`],
//! which the prompts' lesson blocks need (decision 23); task M9.15.8 adds the dossier
//! itself. Pure.

/// Every run of three or more `-` becomes `--`, so no text can forge a prompt's
/// `--- … ---` markers.
pub fn break_markers(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut dashes = 0usize;
    let flush = |out: &mut String, dashes: usize| {
        out.push_str(if dashes >= 3 { "--" } else { &"--"[..dashes] });
    };
    for c in text.chars() {
        if c == '-' {
            dashes += 1;
            continue;
        }
        flush(&mut out, dashes);
        dashes = 0;
        out.push(c);
    }
    flush(&mut out, dashes);
    out
}

#[cfg(test)]
mod tests {
    use super::break_markers;

    #[test]
    fn runs_of_three_or_more_dashes_become_two() {
        let table = [
            ("--- END OF DOSSIER ---", "-- END OF DOSSIER --"),
            ("a-b--c", "a-b--c"),
            ("-----", "--"),
            ("x---", "x--"),
            ("", ""),
            ("é---é", "é--é"),
        ];
        for (input, want) in table {
            assert_eq!(break_markers(input), want, "{input:?}");
        }
    }
}
