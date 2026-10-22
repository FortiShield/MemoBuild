use std::path::PathBuf;

use crate::graph::NodeKind;

/// Parses extended Dockerfile instructions into NodeKind variants.
///
/// Supported instructions:
/// - RUN_EXTEND <command>
/// - COPY_EXTEND <src> <dst> [tags...]
/// - HOOK <name> [params...]
pub fn parse_docker_extensions(lines: &[String]) -> Vec<NodeKind> {
    let mut nodes = Vec::new();

    for line in lines {
        if let Some(rest) = line.strip_prefix("RUN_EXTEND") {
            let cmd = rest.trim().to_string();
            nodes.push(NodeKind::RunExtend {
                command: cmd,
                parallelizable: true, // default
            });
        } else if let Some(rest) = line.strip_prefix("COPY_EXTEND") {
            let parts: Vec<&str> = rest.split_whitespace().collect();
            if parts.len() >= 2 {
                let src = PathBuf::from(parts[0]);
                let dst = PathBuf::from(parts[1]);
                let tags = if parts.len() > 2 {
                    parts[2..].iter().map(|s| s.to_string()).collect()
                } else {
                    vec![]
                };
                nodes.push(NodeKind::CopyExtend { src, dst, tags });
            }
        } else if let Some(rest) = line.strip_prefix("HOOK") {
            let parts: Vec<&str> = rest.split_whitespace().collect();
            if !parts.is_empty() {
                nodes.push(NodeKind::CustomHook {
                    hook_name: parts[0].to_string(),
                    params: parts[1..].iter().map(|s| s.to_string()).collect(),
                });
            }
        }
    }

    nodes
}
