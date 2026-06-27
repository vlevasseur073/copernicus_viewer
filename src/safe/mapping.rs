use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::zarr::tree::{ZarrNodeKind, ZarrTree, ZarrTreeNode};

/// NetCDF variable backing a hierarchy array leaf.
#[derive(Clone, Debug)]
pub struct ArraySource {
    /// Absolute path to the NetCDF file inside the SAFE directory.
    pub nc_path: PathBuf,
    /// Variable name inside the NetCDF file.
    pub var_name: String,
}

/// Parsed Sentinel-3 product mapping (sentineltoolbox JSON format).
#[derive(Clone, Debug)]
pub struct ProductMapping {
    /// EOPF product type code (e.g. `S03SLSLST`).
    #[allow(dead_code)]
    pub product_type: String,
    /// Tree path → NetCDF source.
    pub arrays: HashMap<String, ArraySource>,
    /// Suggested chunk sizes from mapping config (dimension name → size).
    pub chunk_sizes: HashMap<String, u64>,
    /// Geolocation coord paths keyed by logical name (`latitude_in`, …).
    pub coord_paths: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct MappingConfig {
    #[serde(default)]
    chunks: HashMap<String, u64>,
}

/// Load embedded mapping resources for a product type.
pub fn load_mapping(product_type: &str) -> Result<ProductMapping> {
    let map_json = load_map_json(product_type)?;
    let flat_map: HashMap<String, String> =
        serde_json::from_str(&map_json).context("parse mapping JSON")?;

    let config_json = load_config_json(product_type).unwrap_or_else(|_| "{}".to_string());
    let config: MappingConfig = serde_json::from_str(&config_json).unwrap_or(MappingConfig {
        chunks: HashMap::new(),
    });

    let mut arrays = HashMap::new();
    let mut coord_paths = HashMap::new();

    for (src, target) in flat_map {
        let Some((nc_file, var_name)) = src.split_once(':') else {
            continue;
        };
        let tree_path = target_path(&target);
        let source = ArraySource {
            nc_path: PathBuf::from(nc_file),
            var_name: var_name.to_string(),
        };
        if target.starts_with("coords:") {
            let coord_name = target
                .strip_prefix("coords:")
                .unwrap_or(&target)
                .to_string();
            coord_paths.insert(coord_name, tree_path.clone());
        }
        arrays.insert(tree_path, source);
    }

    Ok(ProductMapping {
        product_type: product_type.to_string(),
        arrays,
        chunk_sizes: config.chunks,
        coord_paths,
    })
}

fn load_map_json(product_type: &str) -> Result<String> {
    let file = format!("map/{product_type}.json");
    embedded_json(&file).with_context(|| format!("missing mapping for {product_type}"))
}

fn load_config_json(product_type: &str) -> Result<String> {
    let file = format!("config/{product_type}.json");
    embedded_json(&file)
}

fn embedded_json(relative: &str) -> Result<String> {
    match relative {
        "map/S03OLCEFR.json" => Ok(include_str!("resources/map/S03OLCEFR.json").to_string()),
        "map/S03OLCLFR.json" => Ok(include_str!("resources/map/S03OLCLFR.json").to_string()),
        "map/S03OLCLRR.json" => Ok(include_str!("resources/map/S03OLCLRR.json").to_string()),
        "map/S03OLCRAC.json" => Ok(include_str!("resources/map/S03OLCRAC.json").to_string()),
        "map/S03OLCERR.json" => Ok(include_str!("resources/map/S03OLCERR.json").to_string()),
        "map/S03OLCSPC.json" => Ok(include_str!("resources/map/S03OLCSPC.json").to_string()),
        "map/S03SLSFRP.json" => Ok(include_str!("resources/map/S03SLSFRP.json").to_string()),
        "map/S03SLSLST.json" => Ok(include_str!("resources/map/S03SLSLST.json").to_string()),
        "map/S03SLSRBT.json" => Ok(include_str!("resources/map/S03SLSRBT.json").to_string()),
        "map/S03SYNAOD.json" => Ok(include_str!("resources/map/S03SYNAOD.json").to_string()),
        "map/S03SYNSDR.json" => Ok(include_str!("resources/map/S03SYNSDR.json").to_string()),
        "map/S03SYNV01.json" => Ok(include_str!("resources/map/S03SYNV01.json").to_string()),
        "map/S03SYNV10.json" => Ok(include_str!("resources/map/S03SYNV10.json").to_string()),
        "map/S03SYNVGK.json" => Ok(include_str!("resources/map/S03SYNVGK.json").to_string()),
        "map/S03SYNVGT.json" => Ok(include_str!("resources/map/S03SYNVGT.json").to_string()),
        "config/S03OLCEFR.json" => Ok(include_str!("resources/config/S03OLCEFR.json").to_string()),
        "config/S03OLCLFR.json" => Ok(include_str!("resources/config/S03OLCLFR.json").to_string()),
        "config/S03OLCLRR.json" => Ok(include_str!("resources/config/S03OLCLRR.json").to_string()),
        "config/S03OLCRAC.json" => Ok(include_str!("resources/config/S03OLCRAC.json").to_string()),
        "config/S03OLCERR.json" => Ok(include_str!("resources/config/S03OLCERR.json").to_string()),
        "config/S03OLCSPC.json" => Ok(include_str!("resources/config/S03OLCSPC.json").to_string()),
        "config/S03SLSFRP.json" => Ok(include_str!("resources/config/S03SLSFRP.json").to_string()),
        "config/S03SLSLST.json" => Ok(include_str!("resources/config/S03SLSLST.json").to_string()),
        "config/S03SLSRBT.json" => Ok(include_str!("resources/config/S03SLSRBT.json").to_string()),
        "config/S03SYNAOD.json" => Ok(include_str!("resources/config/S03SYNAOD.json").to_string()),
        "config/S03SYNSDR.json" => Ok(include_str!("resources/config/S03SYNSDR.json").to_string()),
        "config/S03SYNV01.json" => Ok(include_str!("resources/config/S03SYNV01.json").to_string()),
        "config/S03SYNV10.json" => Ok(include_str!("resources/config/S03SYNV10.json").to_string()),
        "config/S03SYNVGK.json" => Ok(include_str!("resources/config/S03SYNVGK.json").to_string()),
        "config/S03SYNVGT.json" => Ok(include_str!("resources/config/S03SYNVGT.json").to_string()),
        _ => bail!("unknown embedded resource: {relative}"),
    }
}

fn target_path(target: &str) -> String {
    if let Some(rest) = target.strip_prefix("coords:") {
        format!("/coords/{rest}")
    } else {
        normalize_tree_path(target)
    }
}

fn normalize_tree_path(path: &str) -> String {
    if path.is_empty() || path == "/" {
        return "/".to_string();
    }
    if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    }
}

/// Build an in-memory hierarchy tree from mapping entries and probed array metadata.
pub fn build_tree(
    mapping: &ProductMapping,
    probe: impl Fn(&ArraySource) -> Result<ArrayMeta>,
    root_attributes: Map<String, Value>,
) -> Result<ZarrTree> {
    let mut paths: Vec<String> = mapping.arrays.keys().cloned().collect();
    paths.sort();

    let mut root = ZarrTreeNode {
        name: "/".to_string(),
        path: "/".to_string(),
        kind: ZarrNodeKind::Group {
            attributes: root_attributes,
        },
        children: Vec::new(),
    };

    for tree_path in paths {
        let source = mapping.arrays.get(&tree_path).expect("path in keys");
        let Ok(meta) = probe(source) else {
            continue;
        };
        insert_array_leaf(&mut root, &tree_path, meta);
    }

    sort_children(&mut root);
    Ok(ZarrTree { root })
}

/// Metadata probed from a NetCDF variable (no data read).
#[derive(Clone, Debug)]
pub struct ArrayMeta {
    pub shape: Vec<u64>,
    pub chunks: Vec<u64>,
    pub dtype: String,
    pub dimension_names: Vec<String>,
    pub attributes: Map<String, Value>,
    pub fill_value: Option<Value>,
}

fn insert_array_leaf(root: &mut ZarrTreeNode, tree_path: &str, meta: ArrayMeta) {
    let segments: Vec<&str> = tree_path.trim_start_matches('/').split('/').collect();
    if segments.is_empty() {
        return;
    }

    if segments.len() == 1 {
        let name = segments[0].to_string();
        root.children.push(leaf_node(&name, tree_path, meta));
        return;
    }

    let head = segments[0];
    let child_path = join_path(&root.path, head);
    let child = root
        .children
        .iter_mut()
        .find(|c| c.name == head && c.is_group());

    if let Some(child) = child {
        insert_array_leaf_inner(child, &child_path, &segments[1..], tree_path, meta);
    } else {
        let mut group = ZarrTreeNode {
            name: head.to_string(),
            path: child_path.clone(),
            kind: ZarrNodeKind::Group {
                attributes: Map::new(),
            },
            children: Vec::new(),
        };
        insert_array_leaf_inner(&mut group, &child_path, &segments[1..], tree_path, meta);
        root.children.push(group);
    }
}

fn insert_array_leaf_inner(
    group: &mut ZarrTreeNode,
    parent_path: &str,
    segments: &[&str],
    full_path: &str,
    meta: ArrayMeta,
) {
    if segments.len() == 1 {
        let name = segments[0].to_string();
        group.children.push(leaf_node(&name, full_path, meta));
        return;
    }

    let head = segments[0];
    let child_path = join_path(parent_path, head);
    let child = group
        .children
        .iter_mut()
        .find(|c| c.name == head && c.is_group());

    if let Some(child) = child {
        insert_array_leaf_inner(child, &child_path, &segments[1..], full_path, meta);
    } else {
        let mut new_group = ZarrTreeNode {
            name: head.to_string(),
            path: child_path.clone(),
            kind: ZarrNodeKind::Group {
                attributes: Map::new(),
            },
            children: Vec::new(),
        };
        insert_array_leaf_inner(&mut new_group, &child_path, &segments[1..], full_path, meta);
        group.children.push(new_group);
    }
}

fn leaf_node(name: &str, path: &str, meta: ArrayMeta) -> ZarrTreeNode {
    ZarrTreeNode {
        name: name.to_string(),
        path: path.to_string(),
        kind: ZarrNodeKind::Array {
            shape: meta.shape,
            chunks: meta.chunks,
            dtype: meta.dtype,
            dimension_names: meta.dimension_names,
            attributes: meta.attributes,
            fill_value: meta.fill_value,
        },
        children: Vec::new(),
    }
}

fn join_path(parent: &str, name: &str) -> String {
    if parent == "/" {
        format!("/{name}")
    } else {
        format!("{parent}/{name}")
    }
}

fn sort_children(node: &mut ZarrTreeNode) {
    node.children
        .sort_by(|a, b| match (a.is_group(), b.is_group()) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.cmp(&b.name),
        });
    for child in &mut node.children {
        sort_children(child);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_sl_lst_mapping() {
        let mapping = load_mapping("S03SLSLST").expect("mapping");
        assert!(mapping.arrays.contains_key("/measurements/lst"));
        assert!(mapping.coord_paths.contains_key("latitude_in"));
        assert_eq!(mapping.coord_paths["latitude_in"], "/coords/latitude_in");
    }
}
