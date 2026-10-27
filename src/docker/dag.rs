use crate::docker::parser::{Instruction, LocatedInstruction};
use crate::graph::{BuildGraph, Node, NodeKind, NodeMetadata};
use anyhow::{bail, Context, Result};
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyOperation {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub destination_is_directory: bool,
    pub source_line: usize,
}

#[derive(Debug, Clone)]
pub struct CompiledBuild {
    pub context_root: PathBuf,
    pub copies: Vec<CopyOperation>,
    pub cmd: Option<Vec<String>>,
    pub entrypoint: Option<Vec<String>>,
    pub graph: BuildGraph,
}

pub fn compile_dockerfile(
    instructions: Vec<LocatedInstruction>,
    context_dir: PathBuf,
) -> Result<CompiledBuild> {
    if instructions.is_empty() {
        bail!("Dockerfile contains no instructions; add `FROM scratch`");
    }
    if !matches!(
        instructions.first().map(|item| &item.instruction),
        Some(Instruction::From { .. })
    ) {
        bail!(
            "Dockerfile line {}: FROM must be the first instruction",
            instructions[0].location.line
        );
    }

    let bases: Vec<_> = instructions
        .iter()
        .filter(|item| matches!(item.instruction, Instruction::From { .. }))
        .collect();
    if bases.len() > 1 {
        bail!(
            "Dockerfile line {}: multi-stage builds are unsupported; \
             the first milestone allows one `FROM scratch` stage",
            bases[1].location.line
        );
    }

    let base = &instructions[0];
    match &base.instruction {
        Instruction::From { image, options } if image == "scratch" && options.is_empty() => {}
        Instruction::From { image, .. } if image == "scratch" => {
            bail!(
                "Dockerfile line {}: FROM options and stage aliases are unsupported; \
                 use exactly `FROM scratch`",
                base.location.line
            );
        }
        Instruction::From { image, .. } => {
            bail!(
                "Dockerfile line {}: base image '{image}' is unsupported. \
                 Only `FROM scratch` is accepted; external image resolution and \
                 filesystem unpacking are not implemented",
                base.location.line
            );
        }
        _ => unreachable!("the first instruction was checked to be FROM"),
    }

    let context_root = std::fs::canonicalize(&context_dir).with_context(|| {
        format!(
            "Failed to resolve build context directory {}",
            context_dir.display()
        )
    })?;
    if !context_root.is_dir() {
        bail!(
            "Build context is not a directory: {}",
            context_root.display()
        );
    }

    let mut nodes = Vec::with_capacity(instructions.len());
    let mut copies = Vec::new();
    let mut cmd = None;
    let mut entrypoint = None;
    for located in &instructions {
        let index = nodes.len();
        let previous = index.checked_sub(1).into_iter().collect();
        let (name, content, kind, source_path) = match &located.instruction {
            Instruction::From { image, .. } => (
                format!("FROM {image}"),
                "FROM scratch".to_string(),
                NodeKind::From,
                None,
            ),
            Instruction::Copy {
                flags,
                sources,
                destination,
            } => {
                let operation = validate_copy(
                    located.location.line,
                    flags,
                    sources,
                    destination,
                    &context_root,
                )?;
                let source_path = context_root.join(&operation.source);
                let content = format!(
                    "COPY {} {}",
                    operation.source.display(),
                    operation.destination.display()
                );
                let kind = NodeKind::Copy {
                    src: operation.source.clone(),
                    dst: operation.destination.clone(),
                };
                copies.push(operation);
                ("COPY".to_string(), content, kind, Some(source_path))
            }
            Instruction::Cmd(arguments) => {
                if cmd.is_some() {
                    bail!(
                        "Dockerfile line {}: duplicate CMD instructions are unsupported",
                        located.location.line
                    );
                }
                cmd = Some(arguments.clone());
                (
                    "CMD".to_string(),
                    serde_json::to_string(arguments)?,
                    NodeKind::Cmd,
                    None,
                )
            }
            Instruction::Entrypoint(arguments) => {
                if entrypoint.is_some() {
                    bail!(
                        "Dockerfile line {}: duplicate ENTRYPOINT instructions are unsupported",
                        located.location.line
                    );
                }
                entrypoint = Some(arguments.clone());
                (
                    "ENTRYPOINT".to_string(),
                    serde_json::to_string(arguments)?,
                    NodeKind::Entrypoint,
                    None,
                )
            }
            Instruction::Unsupported { keyword, .. } => {
                bail!(
                    "Dockerfile line {}: {keyword} is recognized but unsupported by the \
                     first production milestone",
                    located.location.line
                );
            }
            instruction => {
                bail!(
                    "Dockerfile line {}: {} is recognized but unsupported because its \
                     filesystem or image-configuration semantics are not implemented",
                    located.location.line,
                    instruction_name(instruction)
                );
            }
        };

        nodes.push(Node {
            id: index,
            name,
            content,
            kind,
            hash: String::new(),
            dirty: true,
            deps: previous,
            source_path,
            env: Default::default(),
            cache_hit: false,
            metadata: NodeMetadata::default(),
        });
    }

    Ok(CompiledBuild {
        context_root,
        copies,
        cmd,
        entrypoint,
        graph: BuildGraph { nodes },
    })
}

pub fn build_graph_from_instructions(
    instructions: Vec<LocatedInstruction>,
    project_root: PathBuf,
) -> Result<BuildGraph> {
    Ok(compile_dockerfile(instructions, project_root)?.graph)
}

fn instruction_name(instruction: &Instruction) -> &'static str {
    match instruction {
        Instruction::From { .. } => "FROM",
        Instruction::Workdir(_) => "WORKDIR",
        Instruction::Copy { .. } => "COPY",
        Instruction::Run(_) => "RUN",
        Instruction::Env { .. } => "ENV",
        Instruction::Cmd(_) => "CMD",
        Instruction::Entrypoint(_) => "ENTRYPOINT",
        Instruction::Unsupported { keyword, .. } => match keyword.as_str() {
            "GIT" => "GIT",
            "RUN_EXTEND" => "RUN_EXTEND",
            "COPY_EXTEND" => "COPY_EXTEND",
            "HOOK" => "HOOK",
            _ => "unsupported instruction",
        },
    }
}

fn validate_copy(
    line: usize,
    flags: &[String],
    sources: &[String],
    destination: &str,
    context_root: &Path,
) -> Result<CopyOperation> {
    if let Some(from) = flags.iter().find_map(|flag| flag.strip_prefix("--from=")) {
        if from != "0" && from != "scratch" {
            bail!(
                "Dockerfile line {line}: COPY references unknown stage '{from}'; \
                 aliases and multi-stage builds are unsupported"
            );
        }
        bail!(
            "Dockerfile line {line}: COPY --from is unsupported because multi-stage \
             filesystem transfer is not implemented"
        );
    }
    if !flags.is_empty() {
        bail!(
            "Dockerfile line {line}: COPY flags are unsupported; use plain \
             `COPY <source> <destination>`"
        );
    }
    if sources.len() != 1 {
        bail!("Dockerfile line {line}: COPY requires exactly one source and one destination");
    }

    let source_text = &sources[0];
    let source = Path::new(source_text);
    if source.is_absolute()
        || source.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
        || source_text.contains('\\')
    {
        bail!(
            "Dockerfile line {line}: COPY source '{source_text}' must be a relative path inside \
             the build context"
        );
    }
    if source_text.contains('*') || source_text.contains('?') || source_text.contains('[') {
        bail!(
            "Dockerfile line {line}: COPY wildcard sources are unsupported by the \
             first production milestone"
        );
    }

    let source_path = context_root.join(source);
    reject_symlink_components(context_root, source, line)?;
    let resolved = std::fs::canonicalize(&source_path).with_context(|| {
        format!(
            "Dockerfile line {line}: COPY source '{source_text}' does not exist in the build context"
        )
    })?;
    if !resolved.starts_with(context_root) {
        bail!(
            "Dockerfile line {line}: COPY source '{source_text}' resolves outside the build context"
        );
    }
    validate_source_tree(&resolved, line)?;

    let (destination, destination_is_directory) = normalize_destination(destination, line)?;
    Ok(CopyOperation {
        source: source.to_path_buf(),
        destination,
        destination_is_directory,
        source_line: line,
    })
}

fn reject_symlink_components(context_root: &Path, relative: &Path, line: usize) -> Result<()> {
    let mut current = context_root.to_path_buf();
    for component in relative.components() {
        match component {
            Component::CurDir => continue,
            Component::Normal(name) => current.push(name),
            _ => continue,
        }
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                bail!(
                    "Dockerfile line {line}: COPY source '{}' traverses a symlink; \
                     symlink sources are unsupported",
                    relative.display()
                );
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "Dockerfile line {line}: failed to inspect COPY source {}",
                        current.display()
                    )
                });
            }
        }
    }
    Ok(())
}

fn validate_source_tree(source: &Path, line: usize) -> Result<()> {
    let metadata = std::fs::symlink_metadata(source)
        .with_context(|| format!("Dockerfile line {line}: failed to inspect COPY source"))?;
    if !metadata.is_file() && !metadata.is_dir() {
        bail!("Dockerfile line {line}: COPY supports regular files and directories only");
    }
    if metadata.is_dir() {
        for entry in walkdir::WalkDir::new(source).follow_links(false) {
            let entry = entry.with_context(|| {
                format!("Dockerfile line {line}: failed to inspect COPY directory")
            })?;
            let file_type = entry.file_type();
            if file_type.is_symlink() {
                bail!(
                    "Dockerfile line {line}: COPY source '{}' contains a symlink; \
                     symlink sources are unsupported",
                    entry.path().display()
                );
            }
            if !file_type.is_file() && !file_type.is_dir() {
                bail!("Dockerfile line {line}: COPY supports regular files and directories only");
            }
        }
    }
    Ok(())
}

fn normalize_destination(destination: &str, line: usize) -> Result<(PathBuf, bool)> {
    if destination.contains('\\') || destination.as_bytes().contains(&0) {
        bail!("Dockerfile line {line}: COPY destination must use normalized image paths");
    }
    let is_directory = destination.ends_with('/')
        || destination.ends_with("/.")
        || destination == "."
        || destination == "/";
    let mut components = Vec::new();
    for component in destination.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                if components.pop().is_none() {
                    bail!(
                        "Dockerfile line {line}: COPY destination '{destination}' escapes \
                         the image filesystem root"
                    );
                }
            }
            value => components.push(value),
        }
    }
    let normalized = components.into_iter().collect::<PathBuf>();
    Ok((normalized, is_directory))
}
