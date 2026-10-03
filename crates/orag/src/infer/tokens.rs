//! Pure token-sequence helpers shared by inference backends.

use crate::error::{OragError, Result};

/// Guarantees the sequence ends with exactly one EOS when `required`
/// (Qwen3-Embedding last-token pooling). A duplicated EOS means the tokenizer
/// configuration already appends one and the caller appended another: reject.
pub fn ensure_single_trailing_eos<T: PartialEq + Copy>(
    mut tokens: Vec<T>,
    eos: T,
    required: bool,
) -> Result<Vec<T>> {
    let trailing = tokens.iter().rev().take_while(|t| **t == eos).count();
    if trailing > 1 {
        return Err(OragError::Model(format!(
            "tokenizer produced {trailing} trailing EOS tokens"
        )));
    }
    if required && trailing == 0 {
        tokens.push(eos);
    }
    Ok(tokens)
}

/// Rejects inputs longer than the model accepts. Silent truncation would embed
/// only the beginning of a chunk while citing all of it.
pub fn check_input_length<T>(tokens: &[T], max: usize) -> Result<()> {
    if tokens.len() > max {
        return Err(OragError::Model(format!(
            "embedding input of {} tokens exceeds the model limit of {max}",
            tokens.len()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const EOS: u32 = 2;

    #[test]
    fn appends_missing_eos_when_required() {
        assert_eq!(
            ensure_single_trailing_eos(vec![5, 6], EOS, true).unwrap(),
            vec![5, 6, EOS]
        );
    }

    #[test]
    fn keeps_single_eos_and_leaves_optional_case_alone() {
        assert_eq!(
            ensure_single_trailing_eos(vec![5, EOS], EOS, true).unwrap(),
            vec![5, EOS]
        );
        assert_eq!(
            ensure_single_trailing_eos(vec![5], EOS, false).unwrap(),
            vec![5]
        );
    }

    #[test]
    fn rejects_duplicated_eos() {
        assert!(ensure_single_trailing_eos(vec![5, EOS, EOS], EOS, true).is_err());
    }

    #[test]
    fn overlong_input_is_an_error_not_a_truncation() {
        assert!(check_input_length(&[1, 2, 3], 3).is_ok());
        assert!(check_input_length(&[1, 2, 3, 4], 3).is_err());
    }
}
