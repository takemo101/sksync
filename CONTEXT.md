# sksync

`sksync` manages reusable Agent Skills across multiple coding agents while keeping skill sources, local dependencies, and agent placement explicit.

## Language

**Skill**:
Reusable instructions, tool descriptions, templates, or helper files that an agent can load.
_Avoid_: package, plugin, extension

**Agent**:
A coding assistant runtime that can load skills from one or more configured directories.
_Avoid_: client, target, tool

**Source**:
The origin of a skill body or bundle manifest, such as a GitHub location, skills.sh URL, or local directory.
_Avoid_: registry package, marketplace entry

**Dependency**:
A local project or user-level choice to install a skill from a source and make it available to selected agents.
_Avoid_: installed bundle, runtime package

**Bundle**:
A curated install set that names multiple bundle entries for sharing a skill setup.
_Avoid_: runtime folder, agent-visible group, package

**Bundle entry**:
A named skill reference inside a bundle. The entry name is the local skill name the bundle proposes.
_Avoid_: dependency, installed skill

**Bundle provenance**:
Local metadata saying a dependency was installed or adopted through a specific bundle.
_Avoid_: bundle ownership, lockfile state

**Target directory**:
A directory where an agent reads linked skills.
_Avoid_: target agent, agent selection

**Dependency agents**:
The selected agents that receive links for a dependency.
_Avoid_: target agents

## Example dialogue

Developer: “I want to share our review workflow.”

Domain expert: “Create a bundle with one bundle entry per skill. The bundle does not become an agent-visible folder.”

Developer: “When someone adds the bundle, what is stored locally?”

Domain expert: “Each bundle entry becomes a normal dependency. The dependency records bundle provenance so later bundle operations can tell which bundle introduced or adopted it.”

Developer: “If the bundle later adds another skill, where should that skill be linked?”

Domain expert: “Use the dependency agents from the existing dependencies with the same bundle provenance. If there are none, ask the user to choose agents.”
