use crate::CoreError;
use karapace_store::EnvState;

pub fn validate_transition(from: EnvState, to: EnvState) -> Result<(), CoreError> {
    let valid = matches!(
        (from, to),
        (
            EnvState::Defined
                | EnvState::Built
                | EnvState::Running
                | EnvState::Frozen
                | EnvState::Archived,
            EnvState::Built
        ) | (
            EnvState::Built,
            EnvState::Running | EnvState::Frozen | EnvState::Archived
        ) | (EnvState::Running, EnvState::Frozen)
            | (EnvState::Frozen, EnvState::Archived)
    );

    if valid {
        Ok(())
    } else {
        Err(CoreError::InvalidTransition {
            from: from.to_string(),
            to: to.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_transitions() {
        assert!(validate_transition(EnvState::Defined, EnvState::Built).is_ok());
        assert!(validate_transition(EnvState::Built, EnvState::Built).is_ok()); // idempotent rebuild
        assert!(validate_transition(EnvState::Built, EnvState::Running).is_ok());
        assert!(validate_transition(EnvState::Built, EnvState::Frozen).is_ok());
        assert!(validate_transition(EnvState::Running, EnvState::Built).is_ok());
        assert!(validate_transition(EnvState::Running, EnvState::Frozen).is_ok());
        assert!(validate_transition(EnvState::Frozen, EnvState::Built).is_ok());
        assert!(validate_transition(EnvState::Built, EnvState::Archived).is_ok());
        assert!(validate_transition(EnvState::Frozen, EnvState::Archived).is_ok());
        assert!(validate_transition(EnvState::Archived, EnvState::Built).is_ok());
    }

    #[test]
    fn invalid_transitions() {
        assert!(validate_transition(EnvState::Defined, EnvState::Running).is_err());
        assert!(validate_transition(EnvState::Defined, EnvState::Frozen).is_err());
        assert!(validate_transition(EnvState::Archived, EnvState::Running).is_err());
        assert!(validate_transition(EnvState::Running, EnvState::Defined).is_err());
        assert!(validate_transition(EnvState::Frozen, EnvState::Running).is_err());
    }
}
