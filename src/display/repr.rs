use serde_json::Value;

use crate::zarr::{ZarrNodeKind, ZarrTreeNode};

pub struct NodeRepr {
    pub title: String,
    pub body: String,
}

pub fn format_node_repr(node: &ZarrTreeNode, product_name: &str) -> NodeRepr {
    match &node.kind {
        ZarrNodeKind::Group { attributes } => format_group_repr(node, attributes, product_name),
        ZarrNodeKind::Array {
            shape,
            chunks,
            dtype,
            dimension_names,
            attributes,
            fill_value,
        } => format_array_repr(
            node,
            shape,
            chunks,
            dtype,
            dimension_names,
            attributes,
            fill_value,
        ),
    }
}

fn format_group_repr(
    node: &ZarrTreeNode,
    attributes: &serde_json::Map<String, Value>,
    product_name: &str,
) -> NodeRepr {
    let is_root = node.path == "/";
    let title = if is_root {
        format!("<xarray.DataTree '{product_name}'>")
    } else {
        format!("<xarray.Dataset / Group '{}'>", node.name)
    };

    let mut lines = Vec::new();

    let group_children: Vec<_> = node.children.iter().filter(|c| c.is_group()).collect();
    let array_children: Vec<_> = node.children.iter().filter(|c| c.is_array()).collect();

    if !group_children.is_empty() {
        lines.push(format!("Groups: ({})", group_children.len()));
        for child in &group_children {
            lines.push(format!("    - {}", child.name));
        }
        lines.push(String::new());
    }

    if !array_children.is_empty() {
        lines.push(format!("Data variables: ({})", array_children.len()));
        for child in &array_children {
            if let ZarrNodeKind::Array {
                shape,
                dtype,
                dimension_names,
                ..
            } = &child.kind
            {
                let dims = format_dims(shape, dimension_names);
                lines.push(format!(
                    "    {}  ({}) {}",
                    child.name,
                    dims,
                    dtype
                ));
            }
        }
        lines.push(String::new());
    }

    if group_children.is_empty() && array_children.is_empty() && !is_root {
        lines.push("Empty group".to_string());
        lines.push(String::new());
    }

    append_attributes(&mut lines, attributes);

    NodeRepr { title, body: lines.join("\n") }
}

fn format_array_repr(
    node: &ZarrTreeNode,
    shape: &[u64],
    chunks: &[u64],
    dtype: &str,
    dimension_names: &[String],
    attributes: &serde_json::Map<String, Value>,
    fill_value: &Option<Value>,
) -> NodeRepr {
    let title = format!("<xarray.DataArray '{}'>", node.name);
    let mut lines = Vec::new();

    let dims = format_dimension_lines(shape, dimension_names);
    lines.push("Dimensions:".to_string());
    for dim_line in dims {
        lines.push(format!("    {dim_line}"));
    }
    lines.push(String::new());

    let dim_tuple = format_dims(shape, dimension_names);
    lines.push(format!(
        "Data variables:\n    {}  ({}) {}",
        node.name, dim_tuple, dtype
    ));
    lines.push(String::new());

    lines.push("Array metadata:".to_string());
    lines.push(format!("    shape:  {:?}", shape));
    if !chunks.is_empty() {
        lines.push(format!("    chunks: {:?}", chunks));
    }
    if let Some(fv) = fill_value {
        lines.push(format!("    fill_value: {}", format_json_value(fv)));
    }
    lines.push(String::new());

    append_attributes(&mut lines, attributes);

    NodeRepr { title, body: lines.join("\n") }
}

fn format_dimension_lines(shape: &[u64], dimension_names: &[String]) -> Vec<String> {
    if shape.is_empty() {
        return vec!["(scalar)".to_string()];
    }

    shape
        .iter()
        .enumerate()
        .map(|(i, size)| {
            let name = dimension_names
                .get(i)
                .filter(|n| *n != "_")
                .map(String::as_str)
                .unwrap_or("dim");
            format!("{name}: {size}")
        })
        .collect()
}

fn format_dims(shape: &[u64], dimension_names: &[String]) -> String {
    if shape.is_empty() {
        return String::new();
    }

    dimension_names
        .iter()
        .take(shape.len())
        .map(|n| if n == "_" { "dim".to_string() } else { n.clone() })
        .collect::<Vec<_>>()
        .join(", ")
}

fn append_attributes(lines: &mut Vec<String>, attributes: &serde_json::Map<String, Value>) {
    if attributes.is_empty() {
        return;
    }

    lines.push("Attributes:".to_string());
    let mut keys: Vec<_> = attributes.keys().collect();
    keys.sort();

    for key in keys {
        let value = &attributes[key];
        lines.push(format!("    {key}: {}", format_json_value(value)));
    }
}

fn format_json_value(value: &Value) -> String {
    match value {
        Value::String(s) => format!("'{s}'"),
        Value::Array(arr) if arr.len() <= 6 => {
            let inner: Vec<String> = arr.iter().map(format_json_value).collect();
            format!("[{}]", inner.join(", "))
        }
        Value::Array(arr) => format!("[{} items]", arr.len()),
        Value::Object(obj) if obj.is_empty() => "{}".to_string(),
        Value::Object(obj) if obj.len() <= 3 => serde_json::to_string(value).unwrap_or_default(),
        Value::Object(obj) => format!("{{{} keys}}", obj.len()),
        other => other.to_string(),
    }
}
