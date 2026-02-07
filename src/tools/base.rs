use async_trait::async_trait;
use serde_json::{Map, Value, json};

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters(&self) -> Value;

    async fn execute(&self, params: &Map<String, Value>) -> anyhow::Result<String>;

    fn validate_params(&self, params: &Map<String, Value>) -> Vec<String> {
        let schema = self.parameters();
        let schema_type = schema
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("object");
        if schema_type != "object" {
            return vec![format!("schema for {} must be object type", self.name())];
        }
        validate_value(&Value::Object(params.clone()), &schema, "", "parameter")
    }

    fn to_schema(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": self.name(),
                "description": self.description(),
                "parameters": self.parameters(),
            }
        })
    }
}

fn validate_value(value: &Value, schema: &Value, path: &str, fallback_label: &str) -> Vec<String> {
    let schema_type = schema
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("object");
    let label = if path.is_empty() {
        fallback_label
    } else {
        path
    };
    let mut errors = Vec::new();

    match schema_type {
        "string" => {
            if let Some(s) = value.as_str() {
                if let Some(min_len) = schema.get("minLength").and_then(Value::as_u64) {
                    if s.len() < min_len as usize {
                        errors.push(format!("{label} must be at least {min_len} chars"));
                    }
                }
                if let Some(max_len) = schema.get("maxLength").and_then(Value::as_u64) {
                    if s.len() > max_len as usize {
                        errors.push(format!("{label} must be at most {max_len} chars"));
                    }
                }
            } else {
                errors.push(format!("{label} should be string"));
                return errors;
            }
        }
        "integer" => {
            if let Some(num) = value.as_i64() {
                if let Some(min) = schema.get("minimum").and_then(Value::as_i64) {
                    if num < min {
                        errors.push(format!("{label} must be >= {min}"));
                    }
                }
                if let Some(max) = schema.get("maximum").and_then(Value::as_i64) {
                    if num > max {
                        errors.push(format!("{label} must be <= {max}"));
                    }
                }
            } else {
                errors.push(format!("{label} should be integer"));
                return errors;
            }
        }
        "number" => {
            if let Some(num) = value.as_f64() {
                if let Some(min) = schema.get("minimum").and_then(Value::as_f64) {
                    if num < min {
                        errors.push(format!("{label} must be >= {min}"));
                    }
                }
                if let Some(max) = schema.get("maximum").and_then(Value::as_f64) {
                    if num > max {
                        errors.push(format!("{label} must be <= {max}"));
                    }
                }
            } else {
                errors.push(format!("{label} should be number"));
                return errors;
            }
        }
        "boolean" => {
            if !value.is_boolean() {
                errors.push(format!("{label} should be boolean"));
                return errors;
            }
        }
        "array" => {
            if let Some(arr) = value.as_array() {
                if let Some(item_schema) = schema.get("items") {
                    for (idx, item) in arr.iter().enumerate() {
                        let child_path = if path.is_empty() {
                            format!("[{idx}]")
                        } else {
                            format!("{path}[{idx}]")
                        };
                        errors.extend(validate_value(
                            item,
                            item_schema,
                            &child_path,
                            fallback_label,
                        ));
                    }
                }
            } else {
                errors.push(format!("{label} should be array"));
                return errors;
            }
        }
        "object" => {
            if let Some(obj) = value.as_object() {
                let props = schema
                    .get("properties")
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_default();
                if let Some(required) = schema.get("required").and_then(Value::as_array) {
                    for required_key in required.iter().filter_map(Value::as_str) {
                        if !obj.contains_key(required_key) {
                            if path.is_empty() {
                                errors.push(format!("missing required {required_key}"));
                            } else {
                                errors.push(format!("missing required {path}.{required_key}"));
                            }
                        }
                    }
                }
                for (key, item) in obj {
                    if let Some(prop_schema) = props.get(key) {
                        let child_path = if path.is_empty() {
                            key.to_string()
                        } else {
                            format!("{path}.{key}")
                        };
                        errors.extend(validate_value(
                            item,
                            prop_schema,
                            &child_path,
                            fallback_label,
                        ));
                    }
                }
            } else {
                errors.push(format!("{label} should be object"));
                return errors;
            }
        }
        _ => {}
    }

    if let Some(enums) = schema.get("enum").and_then(Value::as_array) {
        if !enums.iter().any(|candidate| candidate == value) {
            errors.push(format!(
                "{label} must be one of {}",
                Value::Array(enums.clone())
            ));
        }
    }

    errors
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::registry::ToolRegistry;

    struct SampleTool;

    #[async_trait]
    impl Tool for SampleTool {
        fn name(&self) -> &str {
            "sample"
        }

        fn description(&self) -> &str {
            "sample tool"
        }

        fn parameters(&self) -> Value {
            json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "minLength": 2 },
                    "count": { "type": "integer", "minimum": 1, "maximum": 10 },
                    "mode": { "type": "string", "enum": ["fast", "full"] },
                    "meta": {
                        "type": "object",
                        "properties": {
                            "tag": { "type": "string" },
                            "flags": {
                                "type": "array",
                                "items": { "type": "string" }
                            }
                        },
                        "required": ["tag"]
                    }
                },
                "required": ["query", "count"]
            })
        }

        async fn execute(&self, _params: &Map<String, Value>) -> anyhow::Result<String> {
            Ok("ok".to_string())
        }
    }

    #[test]
    fn validate_missing_required() {
        let tool = SampleTool;
        let params = json!({ "query": "hi" })
            .as_object()
            .cloned()
            .unwrap_or_default();
        let errors = tool.validate_params(&params).join("; ");
        assert!(errors.contains("missing required count"));
    }

    #[test]
    fn validate_type_and_range() {
        let tool = SampleTool;
        let params = json!({ "query": "hi", "count": 0 })
            .as_object()
            .cloned()
            .unwrap_or_default();
        let errors = tool.validate_params(&params);
        assert!(errors.iter().any(|e| e.contains("count must be >= 1")));

        let params = json!({ "query": "hi", "count": "2" })
            .as_object()
            .cloned()
            .unwrap_or_default();
        let errors = tool.validate_params(&params);
        assert!(errors.iter().any(|e| e.contains("count should be integer")));
    }

    #[test]
    fn validate_enum_and_min_length() {
        let tool = SampleTool;
        let params = json!({ "query": "h", "count": 2, "mode": "slow" })
            .as_object()
            .cloned()
            .unwrap_or_default();
        let errors = tool.validate_params(&params);
        assert!(
            errors
                .iter()
                .any(|e| e.contains("query must be at least 2 chars"))
        );
        assert!(errors.iter().any(|e| e.contains("mode must be one of")));
    }

    #[test]
    fn validate_nested_object_and_array() {
        let tool = SampleTool;
        let params = json!({
            "query": "hi",
            "count": 2,
            "meta": {
                "flags": [1, "ok"]
            }
        })
        .as_object()
        .cloned()
        .unwrap_or_default();
        let errors = tool.validate_params(&params);
        assert!(
            errors
                .iter()
                .any(|e| e.contains("missing required meta.tag"))
        );
        assert!(
            errors
                .iter()
                .any(|e| e.contains("meta.flags[0] should be string"))
        );
    }

    #[test]
    fn validate_ignores_unknown_fields() {
        let tool = SampleTool;
        let params = json!({ "query": "hi", "count": 2, "extra": "x" })
            .as_object()
            .cloned()
            .unwrap_or_default();
        let errors = tool.validate_params(&params);
        assert!(errors.is_empty());
    }

    #[tokio::test]
    async fn registry_returns_validation_error() {
        let mut registry = ToolRegistry::new();
        registry.register(std::sync::Arc::new(SampleTool));
        let result = registry
            .execute("sample", json!({ "query": "hi" }).as_object().unwrap())
            .await;
        assert!(result.contains("Invalid parameters"));
    }
}
