//! The ancestor walk, shared by every OS: the process table and the creation
//! times come from `win.rs` or `mac.rs`, the logic lives here.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcRow {
    pub pid: u32,
    pub parent_pid: u32,
    /// The image file name, no directory (`szExeFile` on Windows).
    pub exe_name: String,
}

/// Pure ancestor walk over a process table. Starts at `pid` (its row and
/// creation time must exist, else empty). Each step needs the parent's row and
/// creation time, and the parent must be strictly older than the child: a
/// younger "parent" is a reused PID, and the walk stops there. Nearest first.
pub fn ancestors(
    rows: &[ProcRow],
    created: impl Fn(u32) -> Option<u64>,
    pid: u32,
    max: usize,
) -> Vec<(ProcRow, u64)> {
    let mut chain = Vec::new();
    let Some(mut child) = rows.iter().find(|r| r.pid == pid) else { return chain };
    let Some(mut child_ft) = created(pid) else { return chain };
    while chain.len() < max {
        let Some(parent) = rows.iter().find(|r| r.pid == child.parent_pid) else { break };
        let Some(parent_ft) = created(parent.pid) else { break };
        if parent_ft >= child_ft {
            break;
        }
        chain.push((parent.clone(), parent_ft));
        child = parent;
        child_ft = parent_ft;
    }
    chain
}

/// The nearest ancestor of `pid` whose image name `matches`, among the first
/// MAX_ANCESTOR_STEPS.
pub fn find_ancestor(
    rows: &[ProcRow],
    created: impl Fn(u32) -> Option<u64>,
    image: impl Fn(u32) -> Option<String>,
    pid: u32,
    matches: fn(&str) -> bool,
) -> Option<crate::AgentProc> {
    ancestors(rows, created, pid, crate::MAX_ANCESTOR_STEPS)
        .into_iter()
        .find(|(row, _)| matches(&row.exe_name))
        .map(|(row, created_ft)| crate::AgentProc {
            pid: row.pid,
            created_ft,
            image: image(row.pid),
        })
}
