use serde_json::{Map, Value};

use crate::zarr::{ZarrNodeKind, ZarrTreeNode};

#[derive(Clone, Debug)]
pub enum AttributeNode {
    Scalar {
        name: String,
        value: String,
    },
    Group {
        name: String,
        children: Vec<AttributeNode>,
    },
    Array {
        name: String,
        children: Vec<AttributeNode>,
    },
}

/// Build a hierarchical attribute tree from root zarr metadata.
///
/// Flat EOPF keys such as `properties:product:type` are merged into nested groups.
/// Nested JSON objects and arrays in attribute values are preserved.
pub fn build_attribute_tree(attrs: &Map<String, Value>) -> Vec<AttributeNode> {
    if attrs.is_empty() {
        return Vec::new();
    }
    let merged = merge_colon_keys(attrs);
    object_to_nodes(&merged)
}

pub fn render_attribute_tree(ui: &mut egui::Ui, nodes: &[AttributeNode], id_prefix: &str) {
    for (index, node) in nodes.iter().enumerate() {
        render_attribute_node(ui, node, &format!("{id_prefix}/{index}"));
    }
}

fn render_attribute_node(ui: &mut egui::Ui, node: &AttributeNode, id_path: &str) {
    match node {
        AttributeNode::Scalar { name, value } => {
            ui.horizontal(|ui| {
                ui.label(format!("{name}:"));
                ui.monospace(value);
            });
        }
        AttributeNode::Group { name, children } => {
            if children.is_empty() {
                ui.label(format!("📁 {name} (empty)"));
                return;
            }

            let label = if is_array_index(name) {
                format!("📄 [{}] {}", name, summarize_group(children))
            } else {
                format!("📁 {name}")
            };
            let default_open = name == "properties" || name == "extent" || name == "links";
            egui::CollapsingHeader::new(label)
                .id_salt(id_path)
                .default_open(default_open)
                .show(ui, |ui| {
                    render_attribute_tree(ui, children, id_path);
                });
        }
        AttributeNode::Array { name, children } => {
            if children.is_empty() {
                ui.label(format!("📋 {name} []"));
                return;
            }

            let label = format!("📋 {name} [{}]", children.len());
            egui::CollapsingHeader::new(label)
                .id_salt(id_path)
                .default_open(false)
                .show(ui, |ui| {
                    render_attribute_tree(ui, children, id_path);
                });
        }
    }
}

fn merge_colon_keys(attrs: &Map<String, Value>) -> Map<String, Value> {
    let mut root = Map::new();
    for (key, value) in attrs {
        let segments: Vec<&str> = key.split(':').collect();
        insert_path(&mut root, &segments, value.clone());
    }
    root
}

fn insert_path(map: &mut Map<String, Value>, segments: &[&str], value: Value) {
    if segments.is_empty() {
        return;
    }
    if segments.len() == 1 {
        merge_value(map, segments[0].to_string(), value);
        return;
    }

    let key = segments[0].to_string();
    let entry = map
        .entry(key)
        .or_insert_with(|| Value::Object(Map::new()));
    if let Value::Object(obj) = entry {
        insert_path(obj, &segments[1..], value);
    }
}

fn merge_value(map: &mut Map<String, Value>, key: String, value: Value) {
    match map.get(&key) {
        None => {
            map.insert(key, value);
        }
        Some(Value::Object(_)) if value.is_object() => {
            let Value::Object(incoming) = value else {
                map.insert(key, value);
                return;
            };
            if let Some(Value::Object(existing)) = map.get_mut(&key) {
                for (child_key, child_value) in incoming {
                    merge_value(existing, child_key, child_value);
                }
            }
        }
        _ => {
            map.insert(key, value);
        }
    }
}

fn object_to_nodes(obj: &Map<String, Value>) -> Vec<AttributeNode> {
    let mut keys: Vec<_> = obj.keys().collect();
    keys.sort();
    keys.into_iter()
        .filter_map(|key| value_to_node(key, &obj[key]))
        .collect()
}

fn value_to_node(name: &str, value: &Value) -> Option<AttributeNode> {
    match value {
        Value::Null => Some(AttributeNode::Scalar {
            name: name.to_string(),
            value: "null".to_string(),
        }),
        Value::Bool(value) => Some(AttributeNode::Scalar {
            name: name.to_string(),
            value: value.to_string(),
        }),
        Value::Number(value) => Some(AttributeNode::Scalar {
            name: name.to_string(),
            value: value.to_string(),
        }),
        Value::String(value) => Some(AttributeNode::Scalar {
            name: name.to_string(),
            value: format!("'{value}'"),
        }),
        Value::Array(values) => {
            if values.is_empty() {
                return Some(AttributeNode::Scalar {
                    name: name.to_string(),
                    value: "[]".to_string(),
                });
            }

            if values.len() <= 6 && values.iter().all(is_compact_scalar) {
                let rendered = values
                    .iter()
                    .map(format_compact_scalar)
                    .collect::<Vec<_>>()
                    .join(", ");
                return Some(AttributeNode::Scalar {
                    name: name.to_string(),
                    value: format!("[{rendered}]"),
                });
            }

            let children = values
                .iter()
                .enumerate()
                .filter_map(|(index, value)| value_to_node(&index.to_string(), value))
                .collect();

            Some(AttributeNode::Array {
                name: name.to_string(),
                children,
            })
        }
        Value::Object(obj) => Some(AttributeNode::Group {
            name: name.to_string(),
            children: object_to_nodes(obj),
        }),
    }
}

fn is_array_index(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_digit())
}

fn summarize_group(children: &[AttributeNode]) -> String {
    children
        .iter()
        .filter_map(|node| match node {
            AttributeNode::Scalar { name, value } => Some(format!("{name}={value}")),
            _ => None,
        })
        .take(3)
        .collect::<Vec<_>>()
        .join(", ")
}

fn is_compact_scalar(value: &Value) -> bool {
    matches!(
        value,
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_)
    )
}

fn format_compact_scalar(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => format!("'{value}'"),
        other => other.to_string(),
    }
}

/// Build the root product attribute tree for the inspector.
pub fn parse_root_attributes(
    node: &ZarrTreeNode,
    root: Option<&ZarrTreeNode>,
) -> Option<Vec<AttributeNode>> {
    let attributes = if node.path == "/" {
        match &node.kind {
            ZarrNodeKind::Group { attributes } => attributes,
            _ => return None,
        }
    } else if let Some(root) = root {
        match &root.kind {
            ZarrNodeKind::Group { attributes } => attributes,
            _ => return None,
        }
    } else {
        return None;
    };

    if attributes.is_empty() {
        return None;
    }

    Some(build_attribute_tree(attributes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn merges_colon_separated_keys_into_nested_groups() {
        let attrs = json!({
            "stac_version": "1.1.0",
            "id": "sample",
            "properties:product:type": "OLCEFR",
            "properties:description": "test"
        })
        .as_object()
        .unwrap()
        .clone();

        let tree = build_attribute_tree(&attrs);
        let properties = tree
            .iter()
            .find_map(|node| match node {
                AttributeNode::Group { name, children } if name == "properties" => {
                    Some(children.clone())
                }
                _ => None,
            })
            .expect("properties group");

        assert!(properties.iter().any(|node| match node {
            AttributeNode::Group { name, children } if name == "product" => children.iter().any(
                |child| matches!(
                    child,
                    AttributeNode::Scalar { name, value }
                        if name == "type" && value == "'OLCEFR'"
                )
            ),
            _ => false,
        }));
    }

    #[test]
    fn preserves_nested_objects_and_arrays() {
        let attrs = json!({
            "extent": {
                "spatial": {
                    "bbox": [-5.0, 45.0, 1.0, 48.0]
                }
            },
            "links": [
                {"rel": "self", "href": "https://example.test/item"},
                {"rel": "collection", "href": "https://example.test/collection"}
            ]
        })
        .as_object()
        .unwrap()
        .clone();

        let tree = build_attribute_tree(&attrs);
        assert!(tree.iter().any(|node| matches!(
            node,
            AttributeNode::Group { name, .. } if name == "extent"
        )));
        assert!(tree.iter().any(|node| matches!(
            node,
            AttributeNode::Array { name, children } if name == "links" && children.len() == 2
        )));
    }

    #[test]
    fn merges_colon_keys_with_existing_nested_object() {
        let attrs = json!({
            "properties": {
                "platform": "Sentinel-3"
            },
            "properties:datetime": "2024-06-01T12:00:00Z"
        })
        .as_object()
        .unwrap()
        .clone();

        let tree = build_attribute_tree(&attrs);
        let properties = tree
            .iter()
            .find_map(|node| match node {
                AttributeNode::Group { name, children } if name == "properties" => {
                    Some(children.clone())
                }
                _ => None,
            })
            .expect("merged properties");

        assert_eq!(properties.len(), 2);
    }
}
