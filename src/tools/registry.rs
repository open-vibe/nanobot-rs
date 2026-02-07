use crate::tools::base::Tool;
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::sync::Arc;

pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        self.tools.insert(tool.name().to_string(), tool);
    }

    pub fn unregister(&mut self, name: &str) {
        self.tools.remove(name);
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).cloned()
    }

    pub fn has(&self, name: &str) -> bool {
        self.tools.contains_key(name)
    }

    pub fn get_definitions(&self) -> Vec<Value> {
        self.tools.values().map(|tool| tool.to_schema()).collect()
    }

    pub async fn execute(&self, name: &str, params: &Map<String, Value>) -> String {
        let Some(tool) = self.tools.get(name) else {
            return format!("Error: Tool '{name}' not found");
        };

        let errors = tool.validate_params(params);
        if !errors.is_empty() {
            return format!(
                "Error: Invalid parameters for tool '{name}': {}",
                errors.join("; ")
            );
        }

        match tool.execute(params).await {
            Ok(output) => output,
            Err(err) => format!("Error executing {name}: {err}"),
        }
    }

    pub fn tool_names(&self) -> Vec<String> {
        self.tools.keys().cloned().collect()
    }

    pub fn len(&self) -> usize {
        self.tools.len()
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}
