//! Experimental: the local generator judges whether an answer only states
//! what its cited sources say. Off by default and never used by any gate
//! (D-018): a model grading its own family's answers is a weak signal.

use std::ops::ControlFlow;

use crate::error::Result;
use crate::infer::{ChatMessage, GenerationRequest, Generator, Role, SamplerProfile};

const QUESTION: &str = "Does the answer only state what the cited sources say? Answer YES or NO.";

/// `Some(true)` for YES, `Some(false)` for NO, `None` when the reply is
/// neither (counted as unjudged, not as NO).
pub fn judge(
    generator: &dyn Generator,
    answer: &str,
    cited_texts: &[String],
) -> Result<Option<bool>> {
    let sources = cited_texts
        .iter()
        .enumerate()
        .map(|(i, text)| format!("Source {}:\n{text}", i + 1))
        .collect::<Vec<_>>()
        .join("\n\n");
    let request = GenerationRequest {
        messages: vec![ChatMessage {
            role: Role::User,
            content: format!("{sources}\n\nAnswer:\n{answer}\n\n{QUESTION}"),
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
    let word: String = reply
        .trim_start()
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
        assert_eq!(verdict("Nothing to judge"), None);
        assert_eq!(verdict(""), None);
    }

    #[test]
    fn the_judge_sees_the_sources_and_the_answer() {
        let generator = FakeGenerator::new("YES");
        let verdict = judge(
            &generator,
            "İade 14 gün [1].",
            &["14 gün içinde iade".into()],
        )
        .unwrap();
        assert_eq!(verdict, Some(true));
        let prompt = &generator.last_request().unwrap().messages[0].content;
        assert!(
            prompt.contains("14 gün içinde iade") && prompt.contains("İade 14 gün [1]."),
            "{prompt}"
        );
    }
}
