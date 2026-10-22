use std::collections::HashMap;
use std::path::Path;

use oci_spec::runtime::*;

pub fn build_spec(cmd: &str, env: &HashMap<String, String>, rootfs: &Path) -> Spec {
    let process = ProcessBuilder::default()
        .args(vec!["/bin/sh".into(), "-c".into(), cmd.into()])
        .env(
            env.iter()
                .map(|(k, v)| format!("{}={}", k, v))
                .collect::<Vec<_>>(),
        )
        .cwd("/workspace")
        .terminal(false)
        .build()
        .unwrap();

    let root = RootBuilder::default()
        .path(rootfs.to_path_buf())
        .readonly(false)
        .build()
        .unwrap();

    let namespaces = vec![
        LinuxNamespaceBuilder::default()
            .typ(LinuxNamespaceType::Pid)
            .build()
            .unwrap(),
        LinuxNamespaceBuilder::default()
            .typ(LinuxNamespaceType::Mount)
            .build()
            .unwrap(),
        LinuxNamespaceBuilder::default()
            .typ(LinuxNamespaceType::Ipc)
            .build()
            .unwrap(),
        LinuxNamespaceBuilder::default()
            .typ(LinuxNamespaceType::Uts)
            .build()
            .unwrap(),
    ];

    let linux = LinuxBuilder::default()
        .namespaces(namespaces)
        .build()
        .unwrap();

    let mounts = vec![
        MountBuilder::default()
            .destination("/proc")
            .typ("proc")
            .source("proc")
            .build()
            .unwrap(),
        MountBuilder::default()
            .destination("/dev")
            .typ("tmpfs")
            .source("tmpfs")
            .build()
            .unwrap(),
    ];

    SpecBuilder::default()
        .version("1.0.2-dev")
        .process(process)
        .root(root)
        .linux(linux)
        .mounts(mounts)
        .build()
        .unwrap()
}
