use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityState {
    Created,
    Started,
    Resumed,
    Paused,
    Stopped,
    Destroyed,
}

#[derive(Debug, Error, PartialEq, Eq)]
#[error("invalid Activity lifecycle transition from {from:?} to {to:?}")]
pub struct LifecycleError {
    pub from: ActivityState,
    pub to: ActivityState,
}

#[derive(Debug, Clone)]
pub struct ActivityLifecycle {
    state: ActivityState,
}

impl ActivityLifecycle {
    #[must_use]
    pub fn new() -> Self {
        Self { state: ActivityState::Created }
    }

    #[must_use]
    pub fn state(&self) -> ActivityState {
        self.state
    }

    pub fn transition(&mut self, target: ActivityState) -> Result<ActivityState, LifecycleError> {
        let valid = matches!(
            (self.state, target),
            (
                ActivityState::Created | ActivityState::Stopped,
                ActivityState::Started | ActivityState::Destroyed
            ) | (
                ActivityState::Started | ActivityState::Paused,
                ActivityState::Resumed | ActivityState::Stopped
            ) | (ActivityState::Resumed, ActivityState::Paused)
        );
        if !valid {
            return Err(LifecycleError { from: self.state, to: target });
        }
        self.state = target;
        Ok(target)
    }
}

impl Default for ActivityLifecycle {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follows_android_activity_lifecycle() {
        let mut lifecycle = ActivityLifecycle::new();
        for state in [
            ActivityState::Started,
            ActivityState::Resumed,
            ActivityState::Paused,
            ActivityState::Stopped,
            ActivityState::Destroyed,
        ] {
            lifecycle.transition(state).expect("valid transition");
        }
        assert_eq!(lifecycle.state(), ActivityState::Destroyed);
    }

    #[test]
    fn rejects_skipping_pause() {
        let mut lifecycle = ActivityLifecycle::new();
        lifecycle.transition(ActivityState::Started).expect("start");
        lifecycle.transition(ActivityState::Resumed).expect("resume");
        assert!(lifecycle.transition(ActivityState::Stopped).is_err());
    }
}
