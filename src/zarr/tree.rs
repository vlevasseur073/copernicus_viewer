use serde_json::{Map, Value};
use zarrs::group::GroupMetadata;
use zarrs::hierarchy::NodeMetadata;
use zarrs::metadata::ArrayMetadata;
use zarrs::node::NodePath;

/// In-memory hierarchy of an EOPF Zarr product.
#[derive(Clone, Debug)]
pub struct ZarrTree {
    /// Root group node (`path == "/"`).
    pub root: ZarrTreeNode,
}

/// A group or array node in the product hierarchy.
#[derive(Clone, Debug)]
pub struct ZarrTreeNode {
    /// Final path segment (e.g. `"lst"`).
    pub name: String,
    /// Absolute hierarchy path (e.g. `"/measurements/lst"`).
    pub path: String,
    /// Group or array metadata for this node.
    pub kind: ZarrNodeKind,
    /// Child groups and arrays, sorted with groups before arrays.
    pub children: Vec<ZarrTreeNode>,
}

/// Metadata carried for a hierarchy node.
#[derive(Clone, Debug)]
pub enum ZarrNodeKind {
    /// Zarr group (`.zgroup`) with optional attributes.
    Group {
        /// Contents of `.zattrs` for this group.
        attributes: Map<String, Value>,
    },
    /// Zarr array (`.zarray`) with shape, chunking, and CF-style attributes.
    Array {
        /// Logical array shape.
        shape: Vec<u64>,
        /// Chunk shape along each dimension.
        chunks: Vec<u64>,
        /// Zarr data type name (e.g. `"float32"`).
        dtype: String,
        /// Named dimensions when present in Zarr v3 metadata.
        dimension_names: Vec<String>,
        /// Contents of `.zattrs` for this array.
        attributes: Map<String, Value>,
        /// Declared fill / missing value, if any.
        fill_value: Option<Value>,
    },
}

impl ZarrTreeNode {
    /// Returns `true` when this node is a Zarr array leaf.
    pub fn is_array(&self) -> bool {
        matches!(self.kind, ZarrNodeKind::Array { .. })
    }

    /// Returns `true` when this node is a Zarr group.
    pub fn is_group(&self) -> bool {
        matches!(self.kind, ZarrNodeKind::Group { .. })
    }

    /// Look up a node by hierarchy path (with or without a leading `/`).
    pub fn find_by_path(&self, path: &str) -> Option<&ZarrTreeNode> {
        let normalized = normalize_path(path);
        if self.path == normalized {
            return Some(self);
        }
        for child in &self.children {
            if let Some(found) = child.find_by_path(path) {
                return Some(found);
            }
        }
        None
    }
}

/// Build a [`ZarrTree`] from zarrs hierarchy traversal output.
pub fn build_tree(nodes: &[(NodePath, NodeMetadata)]) -> ZarrTree {
    let mut root = ZarrTreeNode {
        name: "/".to_string(),
        path: "/".to_string(),
        kind: ZarrNodeKind::Group {
            attributes: Map::new(),
        },
        children: Vec::new(),
    };

    let mut sorted: Vec<_> = nodes
        .iter()
        .map(|(p, m)| (p.as_str().to_string(), m.clone()))
        .collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));

    for (path, metadata) in sorted {
        if path == "/" {
            if let NodeMetadata::Group(group_meta) = metadata {
                root.kind = group_kind(&group_meta);
            }
            continue;
        }

        let segments: Vec<&str> = path.trim_start_matches('/').split('/').collect();
        insert_node(&mut root, "/", &segments, metadata);
    }

    sort_children(&mut root);
    ZarrTree { root }
}

/// Attach root-group attributes from opened zarr metadata to the tree root.
pub fn apply_root_metadata(tree: &mut ZarrTree, meta: &GroupMetadata) {
    tree.root.kind = group_kind(meta);
}

fn insert_node(
    root: &mut ZarrTreeNode,
    parent_path: &str,
    segments: &[&str],
    metadata: NodeMetadata,
) {
    if segments.is_empty() {
        return;
    }

    if segments.len() == 1 {
        let name = segments[0].to_string();
        let path = join_path(parent_path, &name);
        let kind = match metadata {
            NodeMetadata::Group(group_meta) => group_kind(&group_meta),
            NodeMetadata::Array(array_meta) => array_kind(&array_meta),
        };
        root.children.push(ZarrTreeNode {
            name,
            path,
            kind,
            children: Vec::new(),
        });
        return;
    }

    let head = segments[0];
    let child_path = join_path(parent_path, head);

    let child = root
        .children
        .iter_mut()
        .find(|c| c.name == head && c.is_group());

    if let Some(child) = child {
        insert_node(child, &child_path, &segments[1..], metadata);
    } else {
        let mut group = ZarrTreeNode {
            name: head.to_string(),
            path: child_path.clone(),
            kind: ZarrNodeKind::Group {
                attributes: Map::new(),
            },
            children: Vec::new(),
        };
        insert_node(&mut group, &child_path, &segments[1..], metadata);
        root.children.push(group);
    }
}

fn group_kind(meta: &GroupMetadata) -> ZarrNodeKind {
    let attributes = match meta {
        GroupMetadata::V3(v3) => v3.attributes.clone(),
        GroupMetadata::V2(v2) => v2.attributes.clone(),
    };
    ZarrNodeKind::Group { attributes }
}

fn array_kind(meta: &ArrayMetadata) -> ZarrNodeKind {
    match meta {
        ArrayMetadata::V3(v3) => {
            let dimension_names = v3
                .dimension_names
                .clone()
                .unwrap_or_default()
                .into_iter()
                .map(|n| n.unwrap_or_else(|| "_".to_string()))
                .collect();
            let chunks = chunk_shape_from_v3(&v3.chunk_grid);
            let fill_value = serde_json::to_value(&v3.fill_value).ok();

            ZarrNodeKind::Array {
                shape: v3.shape.clone(),
                chunks,
                dtype: v3.data_type.name().to_string(),
                dimension_names,
                attributes: v3.attributes.clone(),
                fill_value,
            }
        }
        ArrayMetadata::V2(v2) => {
            let chunks = v2.chunks.iter().map(|c| c.get()).collect();
            let fill_value = serde_json::to_value(&v2.fill_value).ok();

            ZarrNodeKind::Array {
                shape: v2.shape.clone(),
                chunks,
                dtype: v2.dtype.to_string(),
                dimension_names: Vec::new(),
                attributes: v2.attributes.clone(),
                fill_value,
            }
        }
    }
}

fn chunk_shape_from_v3(chunk_grid: &zarrs::metadata::v3::MetadataV3) -> Vec<u64> {
    #[derive(serde::Deserialize)]
    struct RegularChunkGrid {
        chunk_shape: Vec<u64>,
    }

    chunk_grid
        .to_typed_configuration::<RegularChunkGrid>()
        .map(|c| c.chunk_shape)
        .unwrap_or_default()
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

fn join_path(parent: &str, name: &str) -> String {
    if parent == "/" {
        format!("/{name}")
    } else {
        format!("{parent}/{name}")
    }
}

fn normalize_path(path: &str) -> String {
    if path.is_empty() || path == "/" {
        "/".to_string()
    } else if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    }
}

impl ZarrTreeNode {
    /// Visit every node except the synthetic root `/`.
    pub fn visit_nodes(&self, f: &mut impl FnMut(&ZarrTreeNode)) {
        for child in &self.children {
            child.visit_nodes_inner(f);
        }
    }

    fn visit_nodes_inner(&self, f: &mut impl FnMut(&ZarrTreeNode)) {
        f(self);
        for child in &self.children {
            child.visit_nodes_inner(f);
        }
    }

    /// Sorted hierarchy paths for groups and arrays (excluding `/`).
    pub fn collect_paths(&self) -> Vec<String> {
        let mut paths = Vec::new();
        self.visit_nodes(&mut |node| paths.push(node.path.clone()));
        paths.sort();
        paths
    }

    /// Whether this tree has the same node paths as another (isomorphic layout).
    pub fn is_isomorphic_to(&self, other: &ZarrTreeNode) -> bool {
        self.collect_paths() == other.collect_paths()
    }

    /// Returns `true` when the array shape contains a zero dimension.
    pub fn is_empty_array(&self) -> bool {
        matches!(
            &self.kind,
            ZarrNodeKind::Array { shape, .. } if shape.contains(&0)
        )
    }
}
