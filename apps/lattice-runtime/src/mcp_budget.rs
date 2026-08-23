//! Bounded MCP-call admission that preserves a final read-only handoff reserve.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum McpToolClass {
    Execution,
    Observation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct McpBudget {
    limit: u16,
    reserve: u16,
    used: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct McpRejectionReceipt {
    pub code: &'static str,
    pub reason: &'static str,
    pub effect_was_started: bool,
    pub handoff_required: bool,
    pub retry_allowed: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum McpAdmission {
    Permitted,
    Rejected(McpRejectionReceipt),
}

impl McpBudget {
    #[must_use]
    pub const fn new(limit: u16, reserve: u16) -> Option<Self> {
        if limit == 0 || reserve >= limit {
            return None;
        }
        Some(Self {
            limit,
            reserve,
            used: 0,
        })
    }

    #[must_use]
    pub const fn used(self) -> u16 {
        self.used
    }

    #[must_use]
    pub const fn remaining(self) -> u16 {
        self.limit.saturating_sub(self.used)
    }

    #[must_use]
    pub const fn read_only_reserve(self) -> u16 {
        self.reserve
    }

    pub fn admit(&mut self, class: McpToolClass) -> McpAdmission {
        let execution_ceiling = self.limit - self.reserve;
        let permitted = match class {
            McpToolClass::Execution => self.used < execution_ceiling,
            McpToolClass::Observation => self.used < self.limit,
        };
        if permitted {
            self.used += 1;
            McpAdmission::Permitted
        } else {
            let (code, reason) = if self.used >= self.limit {
                (
                    "LATTICE_MCP_SESSION_EXHAUSTED",
                    "The read-only handoff reserve is exhausted; a fresh MCP session is required.",
                )
            } else {
                (
                    "LATTICE_MCP_BUDGET_HANDOFF_REQUIRED",
                    "The execution budget is exhausted; only the reserved read-only handoff calls remain.",
                )
            };
            McpAdmission::Rejected(McpRejectionReceipt {
                code,
                reason,
                effect_was_started: false,
                handoff_required: true,
                retry_allowed: false,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{McpAdmission, McpBudget, McpToolClass};

    #[test]
    fn execution_stops_before_the_read_only_reserve() {
        let mut budget = McpBudget::new(5, 2).expect("valid budget");
        for _ in 0..3 {
            assert_eq!(
                budget.admit(McpToolClass::Execution),
                McpAdmission::Permitted
            );
        }
        let McpAdmission::Rejected(receipt) = budget.admit(McpToolClass::Execution) else {
            panic!("execution must stop");
        };
        assert_eq!(receipt.code, "LATTICE_MCP_BUDGET_HANDOFF_REQUIRED");
        assert!(!receipt.effect_was_started);
        assert!(receipt.handoff_required);
        assert!(!receipt.retry_allowed);
        assert_eq!(budget.remaining(), 2);
    }

    #[test]
    fn reserve_allows_observation_until_the_actual_limit() {
        let mut budget = McpBudget::new(3, 1).expect("valid budget");
        assert_eq!(
            budget.admit(McpToolClass::Execution),
            McpAdmission::Permitted
        );
        assert_eq!(
            budget.admit(McpToolClass::Execution),
            McpAdmission::Permitted
        );
        assert_eq!(
            budget.admit(McpToolClass::Observation),
            McpAdmission::Permitted
        );
        assert!(matches!(
            budget.admit(McpToolClass::Observation),
            McpAdmission::Rejected(_)
        ));
    }
}
