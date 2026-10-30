//! The picture `artsl` ships with: a little language model on wheels, so the
//! first run works with no network, no key and no news. Pure ASCII on purpose.

use crate::art::{Art, Mood};

const FRAME_A: &[&str] = &[
    r"         .-----------------------------------------------.",
    r"         |  >_ artsl                                [x]  |",
    r"         |-----------------------------------------------|",
    r"         |    (o)-------(o)-------(o)                    |",
    r"         |      \      /   \      /      .-----------.   |",
    r"         |       (o)--'     '--(o)       | hello,    |   |",
    r"         |      /      \   /      \      | world_    |   |",
    r"         |    (o)-------(o)-------(o)    '-----------'   |",
    r"         |         \      |      /                       |",
    r"         |          `--[ LLM ]--'    . o O ( news )      |",
    r"         '-----------------------------------------------'",
    r"    _________|_______________________________________________|_____",
    r"   |_________________________________________________________________|",
    r"      (__)       (__)                                 (__)       (__)",
];

const FRAME_B: &[&str] = &[
    r"         .-----------------------------------------------.",
    r"         |  >_ artsl                                [x]  |",
    r"         |-----------------------------------------------|",
    r"         |    (o)-------(o)-------(o)                    |",
    r"         |      \      /   \      /      .-----------.   |",
    r"         |       (o)--'     '--(o)       | hello,    |   |",
    r"         |      /      \   /      \      | world     |   |",
    r"         |    (o)-------(o)-------(o)    '-----------'   |",
    r"         |         \      |      /                       |",
    r"         |          `--[ LLM ]--'    o O . ( news )      |",
    r"         '-----------------------------------------------'",
    r"    _________|_______________________________________________|_____",
    r"   |_________________________________________________________________|",
    r"      (--)       (--)                                 (--)       (--)",
];

const FRAME_C: &[&str] = &[
    r"         .-----------------------------------------------.",
    r"         |  >_ artsl                                [x]  |",
    r"         |-----------------------------------------------|",
    r"         |    (o)-------(o)-------(o)                    |",
    r"         |      \      /   \      /      .-----------.   |",
    r"         |       (o)--'     '--(o)       | hello,    |   |",
    r"         |      /      \   /      \      | world_    |   |",
    r"         |    (o)-------(o)-------(o)    '-----------'   |",
    r"         |         \      |      /                       |",
    r"         |          `--[ LLM ]--'    O . o ( news )      |",
    r"         '-----------------------------------------------'",
    r"    _________|_______________________________________________|_____",
    r"   |_________________________________________________________________|",
    r"      (||)       (||)                                 (||)       (||)",
];

pub fn builtin() -> Art {
    let frames = [FRAME_A, FRAME_B, FRAME_C]
        .iter()
        .map(|f| f.iter().map(|l| l.to_string()).collect())
        .collect();
    let mut art = Art {
        id: "builtin-llm-express".to_string(),
        generated_at: "2026-10-29T00:00:00Z".to_string(),
        model: String::new(),
        title: "The LLM Express".to_string(),
        caption: "Every run after this one is drawn fresh from the day's news.".to_string(),
        headline: String::new(),
        source: String::new(),
        link: String::new(),
        mood: Mood::Orange,
        frames,
    };
    art.normalise().expect("built-in art is well formed");
    art
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_frames_are_aligned_before_normalisation() {
        for frame in [FRAME_A, FRAME_B, FRAME_C] {
            assert_eq!(frame.len(), FRAME_A.len());
            for (a, b) in frame.iter().zip(FRAME_A.iter()) {
                // Frames may differ in characters but never in shape.
                assert_eq!(a.len(), b.len(), "line length drift:\n{a}\n{b}");
            }
        }
    }

    #[test]
    fn builtin_loads() {
        let art = builtin();
        assert_eq!(art.frames.len(), 3);
        assert!(art.width() > 40);
    }
}
