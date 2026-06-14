use std::{
    collections::HashSet,
    ffi::OsStr,
    fs, io,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Process {
    pub process: String,
    pub states: Vec<String>,
    pub transitions: Vec<Transition>,
    #[serde(default)]
    pub steps: Vec<StepDetail>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transition {
    pub from: String,
    pub to: String,
    pub capability: Option<String>,
    pub guards: Option<Vec<String>>,
    pub posting_rule: Option<PostingRule>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepDetail {
    pub state: String,
    pub description: Option<String>,
    #[serde(default)]
    pub fields: Vec<StepField>,
    #[serde(default)]
    pub documents: Vec<String>,
    #[serde(default)]
    pub gates: Vec<String>,
    #[serde(default)]
    pub kpis: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepField {
    pub name: String,
    pub label: String,
    #[serde(rename = "type")]
    pub field_type: String,
    #[serde(default)]
    pub required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PostingRule {
    pub debit: String,
    pub credit: CreditTarget,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CreditTarget {
    Single(String),
    Multiple(Vec<String>),
}

#[derive(Debug, Error)]
pub enum ProcessError {
    #[error("failed to parse process YAML: {0}")]
    Parse(#[from] serde_yaml_ng::Error),

    #[error("failed to read process file {path:?}: {source}")]
    ReadFile { path: PathBuf, source: io::Error },

    #[error("failed to read process directory {path:?}: {source}")]
    ReadDir { path: PathBuf, source: io::Error },

    #[error("process states must not be empty")]
    EmptyStates,

    #[error("transition references unknown state '{state}'")]
    UnknownState { state: String },
}

impl Process {
    pub fn from_yaml(input: &str) -> Result<Self, ProcessError> {
        let process: Self = serde_yaml_ng::from_str(input)?;
        process.validate()?;
        Ok(process)
    }

    pub fn load_dir(dir: &Path) -> Result<Vec<Self>, ProcessError> {
        let mut paths = Vec::new();

        for entry in fs::read_dir(dir).map_err(|source| ProcessError::ReadDir {
            path: dir.to_path_buf(),
            source,
        })? {
            let entry = entry.map_err(|source| ProcessError::ReadDir {
                path: dir.to_path_buf(),
                source,
            })?;
            let path = entry.path();

            if path
                .extension()
                .is_some_and(|extension| extension == OsStr::new("yaml"))
            {
                paths.push(path);
            }
        }

        paths.sort_by_key(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        });

        paths
            .into_iter()
            .map(|path| {
                let yaml = fs::read_to_string(&path).map_err(|source| ProcessError::ReadFile {
                    path: path.clone(),
                    source,
                })?;
                Self::from_yaml(&yaml)
            })
            .collect()
    }

    pub fn validate(&self) -> Result<(), ProcessError> {
        if self.states.is_empty() {
            return Err(ProcessError::EmptyStates);
        }

        let states: HashSet<_> = self.states.iter().map(String::as_str).collect();

        for transition in &self.transitions {
            if !states.contains(transition.from.as_str()) {
                return Err(ProcessError::UnknownState {
                    state: transition.from.clone(),
                });
            }

            if !states.contains(transition.to.as_str()) {
                return Err(ProcessError::UnknownState {
                    state: transition.to.clone(),
                });
            }
        }

        for step in &self.steps {
            if !states.contains(step.state.as_str()) {
                return Err(ProcessError::UnknownState {
                    state: step.state.clone(),
                });
            }
        }

        Ok(())
    }

    pub fn to_mermaid(&self, current_state: Option<&str>) -> String {
        let mut lines = Vec::with_capacity(self.transitions.len() + 4);

        lines.push("stateDiagram-v2".to_string());
        lines.push(format!("    [*] --> {}", self.states[0]));

        for transition in &self.transitions {
            let mut line = format!("    {} --> {}", transition.from, transition.to);

            if let Some(capability) = &transition.capability {
                line.push_str(": ");
                line.push_str(capability);
            }

            lines.push(line);
        }

        if let Some(current_state) = current_state {
            if self.states.iter().any(|state| state == current_state) {
                lines.push("    classDef current font-weight:bold,fill:#fdd".to_string());
                lines.push(format!("    class {current_state} current"));
            }
        }

        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn customer_invoice_yaml() -> String {
        std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../processes/customer_invoice.yaml"),
        )
        .expect("customer invoice YAML should be readable")
    }

    #[test]
    fn renders_customer_invoice_golden_mermaid_diagram() {
        let yaml = customer_invoice_yaml();
        let process =
            Process::from_yaml(yaml.as_str()).expect("customer invoice YAML should parse");

        let expected = [
            "stateDiagram-v2",
            "    [*] --> draft",
            "    draft --> posted: post_invoice",
            "    posted --> paid: register_payment",
            "    draft --> void: void_invoice",
        ]
        .join("\n");

        assert_eq!(process.to_mermaid(None), expected);
    }

    #[test]
    fn current_state_class_is_rendered_for_known_state() {
        let yaml = customer_invoice_yaml();
        let process =
            Process::from_yaml(yaml.as_str()).expect("customer invoice YAML should parse");

        let diagram = process.to_mermaid(Some("posted"));

        assert!(diagram.contains("    classDef current font-weight:bold,fill:#fdd"));
        assert!(diagram.contains("    class posted current"));
    }

    #[test]
    fn validation_fails_when_transition_references_unknown_to_state() {
        let yaml = r#"
process: invalid
states: [draft]
transitions:
  - from: draft
    to: posted
"#;

        let error = Process::from_yaml(yaml).expect_err("unknown state should fail validation");

        assert!(matches!(
            error,
            ProcessError::UnknownState { state } if state == "posted"
        ));
    }

    #[test]
    fn old_yaml_without_steps_parses_with_empty_steps() {
        let yaml = customer_invoice_yaml();
        let process =
            Process::from_yaml(yaml.as_str()).expect("customer invoice YAML should parse");

        assert!(process.steps.is_empty());
    }

    #[test]
    fn rich_yaml_parses_steps_and_fields() {
        let yaml = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../processes/order_to_cash.yaml"),
        )
        .expect("order-to-cash YAML should be readable");

        let process = Process::from_yaml(yaml.as_str()).expect("rich process YAML should parse");

        let credit_check = process
            .steps
            .iter()
            .find(|step| step.state == "credit_check")
            .expect("credit_check step should be present");

        assert!(
            credit_check
                .fields
                .iter()
                .any(|field| field.name == "current_exposure"
                    && field.label == "Current exposure"
                    && field.field_type == "money"
                    && field.required)
        );
    }

    #[test]
    fn bundled_process_yaml_files_parse() {
        let processes = Process::load_dir(
            &Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .join("processes"),
        )
        .expect("bundled process YAML files should parse");

        assert!(
            processes
                .iter()
                .any(|process| process.process == "order_to_cash" && !process.steps.is_empty())
        );
    }

    #[test]
    fn validation_fails_when_step_references_unknown_state() {
        let yaml = r#"
process: invalid
states: [draft]
transitions: []
steps:
  - state: posted
    description: Posted invoice
"#;

        let error = Process::from_yaml(yaml).expect_err("unknown step state should fail");

        assert!(matches!(
            error,
            ProcessError::UnknownState { state } if state == "posted"
        ));
    }
}
