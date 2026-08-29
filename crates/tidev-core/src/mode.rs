use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[serde(alias = "Plan", alias = "PLAN")]
    Plan,
    #[serde(alias = "Build", alias = "BUILD")]
    Build,
}

impl Mode {
    pub fn all() -> &'static [Self] {
        &[Self::Plan, Self::Build]
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Plan => "plan",
            Self::Build => "build",
        }
    }

    pub fn toggle(self) -> Self {
        match self {
            Self::Plan => Self::Build,
            Self::Build => Self::Plan,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Plan => "Plan",
            Self::Build => "Build",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Plan => "Read-only planning mode",
            Self::Build => "Full access build mode",
        }
    }
}

impl std::fmt::Display for Mode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for Mode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "plan" => Ok(Self::Plan),
            "build" => Ok(Self::Build),
            _ => Err(format!("unknown session mode: {s}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mode_serde() {
        assert_eq!(serde_json::to_string(&Mode::Plan).unwrap(), "\"plan\"");
        assert_eq!(serde_json::to_string(&Mode::Build).unwrap(), "\"build\"");

        assert_eq!(
            serde_json::from_str::<Mode>("\"plan\"").unwrap(),
            Mode::Plan
        );
        assert_eq!(
            serde_json::from_str::<Mode>("\"build\"").unwrap(),
            Mode::Build
        );
        assert_eq!(
            serde_json::from_str::<Mode>("\"Plan\"").unwrap(),
            Mode::Plan
        );
        assert_eq!(
            serde_json::from_str::<Mode>("\"Build\"").unwrap(),
            Mode::Build
        );
    }
}
