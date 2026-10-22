use std::path::Path;

use blake3::Hasher;

use crate::graph::BuildGraph;
use crate::hasher::{self, IgnoreRules};

/// Hash a string using BLAKE3 (used for non-filesystem nodes)
pub fn hash_str(input: &str) -> String {
    let mut hasher = Hasher::new();
    hasher.update(input.as_bytes());
    hasher.finalize().to_hex().to_string()
}

/// Load ignore rules respecting precedence:
/// Fix 4 — .dockerignore > .gitignore > empty (Docker's documented behaviour)
fn load_ignore_rules(context_dir: &Path) -> IgnoreRules {
    let dockerignore = context_dir.join(".dockerignore");
    let gitignore = context_dir.join(".gitignore");

    if dockerignore.exists() {
        IgnoreRules::from_file(&dockerignore)
    } else if gitignore.exists() {
        IgnoreRules::from_file(&gitignore)
    } else {
        IgnoreRules::empty()
    }
}

/// Detect changes in the build graph.
/// - COPY nodes with a source_path → real filesystem hash via hasher module
/// - All other nodes → BLAKE3 hash of content string
pub fn detect_changes(graph: &mut BuildGraph) {
    let context_dir = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let ignore = load_ignore_rules(&context_dir);

    for node in graph.nodes.iter_mut() {
        let new_hash = if let Some(ref path) = node.source_path {
            // Real filesystem hashing for COPY nodes
            match hasher::hash_path(path, &ignore) {
                Ok(h) => {
                    // Generate and track manifest for remote execution
                    if let Ok(manifest) = crate::cache_utils::ArtifactManifest::from_dir(path) {
                        node.metadata.input_manifest_hash = Some(manifest.hash());
                    }
                    h
                }
                Err(e) => {
                    eprintln!("⚠️  Hash error for {}: {}", path.display(), e);
                    hash_str(&node.content)
                }
            }
        } else if node.content.starts_with("GIT ") {
            // Git-based hashing for remote repositories
            let parts: Vec<&str> = node.content.split_whitespace().collect();
            if parts.len() >= 2 {
                let url = parts[1];
                match crate::git::get_remote_head_hash(url) {
                    Ok(h) => h,
                    Err(e) => {
                        eprintln!("⚠️  Git hash error for {}: {}", url, e);
                        hash_str(&node.content)
                    }
                }
            } else {
                hash_str(&node.content)
            }
        } else {
            // Command / env / metadata nodes: hash the instruction text
            hash_str(&node.content)
        };

        // Incorporate AI-detected extra source paths into the hash
        let mut final_hash = new_hash;
        if !node.metadata.extra_source_paths.is_empty() {
            let mut hasher = Hasher::new();
            hasher.update(final_hash.as_bytes());
            for extra_path in &node.metadata.extra_source_paths {
                if let Ok(h) = hasher::hash_path(extra_path, &ignore) {
                    hasher.update(h.as_bytes());
                }
            }
            final_hash = hasher.finalize().to_hex().to_string();
        }

        if node.hash != final_hash {
            println!(
                "   - {}: {} -> {}",
                node.name,
                &node.hash[..std::cmp::min(8, node.hash.len())],
                &final_hash[..8]
            );
            node.dirty = true;
            node.hash = final_hash;
        }
    }
}

/// Propagate dirty flags: if a dependency is dirty, mark all dependents dirty too.
pub fn propagate_dirty(graph: &mut BuildGraph) {
    let mut changed = true;
    while changed {
        changed = false;
        for i in 0..graph.nodes.len() {
            let deps_dirty = graph.nodes[i]
                .deps
                .iter()
                .any(|&d| d < graph.nodes.len() && graph.nodes[d].dirty);

            if deps_dirty && !graph.nodes[i].dirty {
                graph.nodes[i].dirty = true;
                changed = true;
            }
        }
    }
}

/// Recompute composite hashes for all nodes using topological order.
/// This ensures that a node's hash reflects its content, dependencies, and environment.
pub fn compute_composite_hashes(graph: &mut BuildGraph, env_fp: &crate::env::EnvFingerprint) {
    let order = graph.topological_order();

    for node_id in order {
        let dep_hashes: Vec<String> = graph.nodes[node_id]
            .deps
            .iter()
            .map(|&d| graph.nodes[d].hash.clone())
            .collect();

        let context_hash = graph.nodes[node_id].metadata.source_content_hash.as_deref();

        let composite_hash =
            graph.nodes[node_id].compute_node_key(&dep_hashes, context_hash, Some(env_fp));

        graph.nodes[node_id].hash = composite_hash;
    }
}

/// Propagate manifests: each node's input manifest is the union of its dependencies' output manifests.
/// Returns a map of manifest_hash -> ArtifactManifest so callers can upload them to the CAS.
pub fn propagate_manifests(
    graph: &mut crate::graph::BuildGraph,
) -> std::collections::HashMap<String, crate::cache_utils::ArtifactManifest> {
    let order = graph.topological_order();
    let mut node_manifests: std::collections::HashMap<usize, crate::cache_utils::ArtifactManifest> =
        std::collections::HashMap::new();
    let mut all_manifests: std::collections::HashMap<String, crate::cache_utils::ArtifactManifest> =
        std::collections::HashMap::new();

    for node_id in order {
        let mut input_manifest = crate::cache_utils::ArtifactManifest { files: Vec::new() };

        // 1. Merge parent output manifests
        for &dep in &graph.nodes[node_id].deps {
            if let Some(parent_manifest) = node_manifests.get(&dep) {
                input_manifest.merge(parent_manifest);
            }
        }

        if !input_manifest.files.is_empty() {
            let h = input_manifest.hash();
            graph.nodes[node_id].metadata.input_manifest_hash = Some(h.clone());
            all_manifests.insert(h, input_manifest.clone());
        }

        // 2. Compute this node's output manifest
        let mut output_manifest = input_manifest.clone();

        // If it's a COPY node, add its specific files
        if let crate::graph::NodeKind::Copy { src, .. } = &graph.nodes[node_id].kind {
            if let Ok(delta) = crate::cache_utils::ArtifactManifest::from_dir(src) {
                output_manifest.merge(&delta);
            }
        }

        let oh = output_manifest.hash();
        graph.nodes[node_id].metadata.output_manifest_hash = Some(oh.clone());
        all_manifests.insert(oh, output_manifest.clone());
        node_manifests.insert(node_id, output_manifest);
    }

    all_manifests
}
