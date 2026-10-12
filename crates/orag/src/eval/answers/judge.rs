//! Experimental: the local generator judges whether an answer only states
//! what its cited sources say. Off by default and never used by any gate
//! (D-018): a model grading its own family's answers is a weak signal.

use std::ops::ControlFlow;

use crate::error::Result;
use crate::infer::{ChatMessage, GenerationRequest, Generator, Role, SamplerProfile};

const QUESTION: &str = "Does the answer only state what the cited sources say? Answer YES or NO.";

/// `Some(true)` for YES, `Some(false)` for NO, `None` when the reply is
/// neither (counted as unjudged, not as NO). `cited` pairs each cited
/// source's own number (the `[n]` the answer uses) with its text.
pub fn judge(
    generator: &dyn Generator,
    answer: &str,
    cited: &[(usize, String)],
) -> Result<Option<bool>> {
    let sources = cited
        .iter()
        .map(|(number, text)| format!("[{number}]\n{text}"))
        .collect::<Vec<_>>()
        .join("\n\n");
    let request = GenerationRequest {
        messages: vec![ChatMessage {
            role: Role::User,
            content: format!("Sources:\n{sources}\n\nAnswer:\n{answer}\n\n{QUESTION}"),
        }],
        max_output_tokens: 4,
        sampler: SamplerProfile::Greedy,
    };
    let mut reply = String::new();
    generator.generate(&request, &mut |piece| {
        reply.push_str(piece);
        ControlFlow::Continue(())
    })?;
    Ok(verdict(&reply))
}

fn verdict(reply: &str) -> Option<bool> {
    // Markdown or quotes around the word (`**YES**`, `"NO"`) do not hide it.
    let word: String = reply
        .trim_start_matches(|c: char| !c.is_alphabetic())
        .chars()
        .take_while(|c| c.is_alphabetic())
        .collect::<String>()
        .to_uppercase();
    match word.as_str() {
        "YES" => Some(true),
        "NO" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infer::fake::FakeGenerator;

    #[test]
    fn the_first_word_of_the_reply_is_the_verdict() {
        assert_eq!(verdict(" Yes, it does."), Some(true));
        assert_eq!(verdict("NO"), Some(false));
        assert_eq!(verdict("**YES**"), Some(true));
        assert_eq!(verdict("- \"No\""), Some(false));
        assert_eq!(verdict("Nothing to judge"), None);
        assert_eq!(verdict(""), None);
    }

    #[test]
    fn the_judge_sees_each_source_under_its_own_number() {
        let generator = FakeGenerator::new("YES");
        // The answer cites [3]: the judge must see that source as [3], not [1].
        let cited = [(3, "14 gün içinde iade".to_string())];
        let verdict = judge(&generator, "İade 14 gün [3].", &cited).unwrap();
        assert_eq!(verdict, Some(true));
        let prompt = &generator.last_request().unwrap().messages[0].content;
        assert!(
            prompt.contains("[3]\n14 gün içinde iade") && prompt.contains("İade 14 gün [3]."),
            "{prompt}"
        );
    }
}
