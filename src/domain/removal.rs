use super::agent::AgentKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillRemovalScope {
    EntireSkill,
    SelectedAgents,
}

pub fn classify_skill_removal(
    existing_agents: &[AgentKind],
    requested_agents: &[AgentKind],
) -> SkillRemovalScope {
    if requested_agents.is_empty() || removes_all_existing_agents(existing_agents, requested_agents)
    {
        SkillRemovalScope::EntireSkill
    } else {
        SkillRemovalScope::SelectedAgents
    }
}

fn removes_all_existing_agents(
    existing_agents: &[AgentKind],
    requested_agents: &[AgentKind],
) -> bool {
    !existing_agents.is_empty()
        && existing_agents
            .iter()
            .all(|agent| requested_agents.iter().any(|requested| requested == agent))
}

#[cfg(test)]
mod tests {
    use super::{classify_skill_removal, SkillRemovalScope};
    use crate::domain::agent::AgentKind;

    #[test]
    fn empty_agent_request_removes_entire_skill() {
        assert_eq!(
            classify_skill_removal(&[AgentKind::Pi], &[]),
            SkillRemovalScope::EntireSkill
        );
    }

    #[test]
    fn requesting_all_existing_agents_removes_entire_skill() {
        assert_eq!(
            classify_skill_removal(
                &[AgentKind::Pi, AgentKind::ClaudeCode],
                &[AgentKind::ClaudeCode, AgentKind::Pi],
            ),
            SkillRemovalScope::EntireSkill
        );
    }

    #[test]
    fn requesting_some_existing_agents_detaches_only_those_agents() {
        assert_eq!(
            classify_skill_removal(&[AgentKind::Pi, AgentKind::ClaudeCode], &[AgentKind::Pi]),
            SkillRemovalScope::SelectedAgents
        );
    }

    #[test]
    fn missing_skill_with_agent_request_keeps_store_error_path() {
        assert_eq!(
            classify_skill_removal(&[], &[AgentKind::Pi]),
            SkillRemovalScope::SelectedAgents
        );
    }
}
