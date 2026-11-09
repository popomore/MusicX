//! Bounded, dependency-free line timing for the original LRC strings.
const MAX_CUES: usize = 2048;
const MAX_BYTES: usize = 128 * 1024;

#[derive(Clone, Default, PartialEq)]
pub struct Cue {
    pub ms: i64,
    pub text: String,
    pub translation: String,
}

#[derive(Default, PartialEq)]
pub struct Timeline {
    pub cues: Vec<Cue>,
}

impl Timeline {
    pub fn parse(original: &str, translation: &str) -> Self {
        let mut cues = parse(original);
        let translations = parse(translation);
        let mut remaining = MAX_BYTES;
        for cue in &mut cues {
            let at = translations.partition_point(|t| t.ms < cue.ms);
            let nearest = [at.checked_sub(1), Some(at)]
                .into_iter()
                .flatten()
                .filter_map(|i| translations.get(i))
                .min_by_key(|t| t.ms.abs_diff(cue.ms));
            if let Some(line) = nearest.filter(|t| t.ms.abs_diff(cue.ms) <= 50) {
                cue.translation = bounded(&line.text, remaining);
                remaining -= cue.translation.len();
            }
        }
        Self { cues }
    }

    pub fn active(&self, seconds: f64) -> Option<usize> {
        if !seconds.is_finite() || seconds < 0.0 {
            return None;
        }
        let ms = (seconds * 1000.0).floor() as i64;
        self.cues.partition_point(|c| c.ms <= ms).checked_sub(1)
    }
}

pub fn stamp(ms: i64) -> String {
    let ms = ms.max(0);
    format!("{}:{:02}.{:03}", ms / 60000, (ms / 1000) % 60, ms % 1000)
}

fn bounded(text: &str, max: usize) -> String {
    let mut end = text.len().min(max);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

fn timestamp(tag: &str) -> Option<i64> {
    let (minutes, seconds) = tag.split_once(':')?;
    if minutes.is_empty() || !minutes.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let (seconds, fraction) = seconds.split_once('.').unwrap_or((seconds, ""));
    if seconds.is_empty()
        || !seconds.bytes().all(|b| b.is_ascii_digit())
        || !fraction.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let seconds: i64 = seconds.parse().ok()?;
    if seconds >= 60 {
        return None;
    }
    let fraction = fraction
        .bytes()
        .take(3)
        .fold(0, |v, b| v * 10 + i64::from(b - b'0'))
        * 10_i64.pow(3 - fraction.len().min(3) as u32);
    minutes
        .parse::<i64>()
        .ok()?
        .checked_mul(60000)?
        .checked_add(seconds * 1000 + fraction)
}

fn words(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    for part in text.split_inclusive('>') {
        if let Some(body) = part.strip_suffix('>')
            && let Some(start) = body.rfind('<')
            && timestamp(&body[start + 1..]).is_some()
        {
            result.push_str(&body[..start]);
        } else {
            result.push_str(part);
        }
    }
    result.trim().to_owned()
}

fn parse(input: &str) -> Vec<Cue> {
    let mut rows = Vec::new();
    let mut budget = MAX_BYTES;
    let mut offset = 0_i64;
    for raw in input.lines().take(MAX_CUES * 2) {
        let mut rest = raw.trim().trim_start_matches('\u{feff}');
        let mut times = Vec::new();
        while let Some(tag) = rest.strip_prefix('[')
            && let Some(end) = tag.find(']')
        {
            let tag_text = &tag[..end];
            if let Some(value) = tag_text.strip_prefix("offset:") {
                if let Ok(value) = value.trim().parse::<i64>()
                    && value.unsigned_abs() <= 3_600_000
                {
                    offset = value;
                }
            } else if let Some(ms) = timestamp(tag_text) {
                times.push(ms);
            }
            rest = tag[end + 1..].trim_start();
            if times.len() >= MAX_CUES {
                rest = "";
                break;
            }
        }
        if times.is_empty() {
            continue;
        }
        let text = words(&bounded(rest, MAX_BYTES));
        for ms in times {
            if rows.len() == MAX_CUES || text.len() > budget {
                break;
            }
            budget -= text.len();
            rows.push(Cue {
                ms,
                text: text.clone(),
                translation: String::new(),
            });
        }
        if rows.len() == MAX_CUES {
            break;
        }
    }
    for row in &mut rows {
        row.ms = row.ms.saturating_sub(offset).max(0);
    }
    rows.sort_by_key(|r| r.ms);
    let mut merged: Vec<Cue> = Vec::with_capacity(rows.len());
    for row in rows {
        if let Some(last) = merged.last_mut()
            && last.ms == row.ms
        {
            if !row.text.is_empty() && last.text != row.text {
                if !last.text.is_empty() {
                    last.text.push('\n');
                }
                last.text.push_str(&row.text);
            }
        } else {
            merged.push(row);
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fractions_multitags_and_backward_seek() {
        let t = Timeline::parse("[ar:test]\n[00:02.35][00:05.007]second\n[00:01]first", "");
        assert_eq!(
            t.cues.iter().map(|c| c.ms).collect::<Vec<_>>(),
            [1000, 2350, 5007]
        );
        assert_eq!(t.active(0.999), None);
        assert_eq!(t.active(2.349), Some(0));
        assert_eq!(t.active(2.35), Some(1));
        assert_eq!(t.active(5.1), Some(2));
        assert_eq!(t.active(1.1), Some(0));
        assert_eq!(t.active(f64::NAN), None);
    }
    #[test]
    fn offset_blanks_duplicates_and_word_markup() {
        let t = Timeline::parse(
            "[00:01.500]<00:01.500>a<00:01.900>b\n[00:02]\n[00:02.5]c\n[00:02.5]d\n[offset:500]",
            "",
        );
        assert_eq!(t.cues[0].text, "ab");
        assert_eq!(t.cues[0].ms, 1000);
        assert!(t.cues[1].text.is_empty());
        assert_eq!(t.active(1.7), Some(1));
        assert_eq!(t.cues[2].text, "c\nd");
    }
    #[test]
    fn translations_align_by_time_not_line_number() {
        let t = Timeline::parse("[00:01]a\n[00:02]b\n[00:03]c", "[00:03.01]三\n[00:01]一");
        assert_eq!(t.cues[0].translation, "一");
        assert!(t.cues[1].translation.is_empty());
        assert_eq!(t.cues[2].translation, "三");
    }
    #[test]
    fn malformed_and_untimed_input_and_resource_bounds() {
        assert!(
            Timeline::parse("ordinary text\n[xx:yy]bad\n[00:60]bad", "")
                .cues
                .is_empty()
        );
        let huge = format!("{}body", "[00:01]".repeat(10000));
        assert!(Timeline::parse(&huge, "").cues.len() <= MAX_CUES);
        let large = (0..4000)
            .map(|i| format!("[{}:{:02}]{}\n", i / 60, i % 60, "海".repeat(100)))
            .collect::<String>();
        let t = Timeline::parse(&large, "");
        assert!(t.cues.len() <= MAX_CUES);
        assert!(t.cues.iter().map(|c| c.text.len()).sum::<usize>() <= MAX_BYTES);
    }
}
