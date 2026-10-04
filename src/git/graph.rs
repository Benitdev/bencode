//! MonoCode `gitGraph.ts` (from VS Code's `scmHistory.ts`, MIT): lays the
//! history out in swimlanes, one 22px row per commit, and gives each row
//! the paths and circles to draw. Paths are SVG-like commands, painted by
//! the Graph view.

use super::sync::{HistoryCommit, HistoryRef};

pub const SWIMLANE_HEIGHT: f32 = 22.0;
pub const SWIMLANE_WIDTH: f32 = 11.0;
const SWIMLANE_CURVE_RADIUS: f32 = 5.0;
const CIRCLE_RADIUS: f32 = 4.0;

/// Lane colours, cycled as branches appear (`COLOR_REGISTRY`).
pub const LANE_COLORS: [u32; 5] = [0xFFB000, 0xDC267F, 0x994F00, 0x40B0A6, 0xB66DFF];
/// The current branch, and its upstream.
pub const REF_COLOR: u32 = 0x75BEFF;
pub const REMOTE_REF_COLOR: u32 = 0xB180D7;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Swimlane {
    pub id: String,
    pub color: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphRef {
    pub name: String,
    pub kind: String,
    pub color: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub sha: String,
    pub parents: Vec<String>,
    pub head: bool,
    pub input: Vec<Swimlane>,
    pub output: Vec<Swimlane>,
    pub refs: Vec<GraphRef>,
}

/// One path command, in row coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Cmd {
    Move(f32, f32),
    Line(f32, f32),
    /// `A r r 0 0 sweep x y`.
    Arc { r: f32, sweep: bool, x: f32, y: f32 },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Path {
    pub cmds: Vec<Cmd>,
    pub color: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Node {
    /// HEAD: a ring.
    Head,
    /// A merge: a larger dot.
    Merge,
    Commit,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RowGraph {
    pub width: f32,
    pub paths: Vec<Path>,
    pub cx: f32,
    pub cy: f32,
    pub node: Node,
    pub color: u32,
}

fn ref_id(r: &HistoryRef) -> String {
    format!("{}:{}", r.kind, r.name)
}

/// The current branch's local ref and its remote counterpart.
fn current_refs(commits: &[HistoryCommit]) -> (Option<String>, Option<String>) {
    let Some(head) = commits.iter().find(|c| c.head) else {
        return (None, None);
    };
    let Some(local) = head.refs.iter().find(|r| r.kind == "local") else {
        return (None, None);
    };
    let remote = head.refs.iter().find(|r| {
        r.kind == "remote" && (r.name == local.name || r.name.ends_with(&format!("/{}", local.name)))
    });
    (Some(ref_id(local)), remote.map(ref_id))
}

/// MonoCode `layoutGitGraph` for `git log --topo-order` (newest first).
pub fn layout(commits: &[HistoryCommit]) -> Vec<Row> {
    let (local, remote) = current_refs(commits);
    let mut colors: std::collections::HashMap<String, Option<u32>> = Default::default();
    if let Some(local) = &local {
        colors.insert(local.clone(), Some(REF_COLOR));
        if let Some(remote) = &remote {
            colors.insert(remote.clone(), Some(REMOTE_REF_COLOR));
        }
    }
    for commit in commits {
        for r in &commit.refs {
            colors.entry(ref_id(r)).or_insert(None);
        }
    }
    let label_color = |refs: &[HistoryRef]| refs.iter().find_map(|r| colors.get(&ref_id(r)).copied().flatten());
    let mut color_index: isize = -1;
    let mut rows: Vec<Row> = Vec::with_capacity(commits.len());
    for commit in commits {
        let input: Vec<Swimlane> = rows.last().map_or_else(Vec::new, |r| r.output.clone());
        let mut output: Vec<Swimlane> = Vec::new();
        let parents: Vec<String> = commit.parents.iter().filter(|p| !p.is_empty()).cloned().collect();
        let mut first_parent_added = false;
        if !parents.is_empty() {
            for node in &input {
                if node.id == commit.sha {
                    if !first_parent_added {
                        output.push(Swimlane {
                            id: parents[0].clone(),
                            color: label_color(&commit.refs).unwrap_or(node.color),
                        });
                        first_parent_added = true;
                    }
                    continue;
                }
                output.push(node.clone());
            }
        }
        for (i, parent) in parents.iter().enumerate().skip(usize::from(first_parent_added)) {
            let color = if i == 0 {
                label_color(&commit.refs)
            } else {
                commits
                    .iter()
                    .find(|c| &c.sha == parent)
                    .and_then(|c| label_color(&c.refs))
            };
            let color = color.unwrap_or_else(|| {
                color_index = (color_index + 1).rem_euclid(LANE_COLORS.len() as isize);
                LANE_COLORS[color_index as usize]
            });
            output.push(Swimlane {
                id: parent.clone(),
                color,
            });
        }
        let circle = input
            .iter()
            .position(|n| n.id == commit.sha)
            .unwrap_or(input.len());
        let lane_color = output
            .get(circle)
            .or(input.get(circle))
            .map_or(REF_COLOR, |l| l.color);
        let mut refs: Vec<GraphRef> = commit
            .refs
            .iter()
            .map(|r| GraphRef {
                name: r.name.clone(),
                kind: r.kind.clone(),
                color: match colors.get(&ref_id(r)) {
                    Some(Some(color)) => Some(*color),
                    Some(None) => Some(lane_color),
                    None => None,
                },
            })
            .collect();
        // MonoCode `compareRefs`: current branch, its upstream, coloured.
        let order = |r: &GraphRef| {
            let id = format!("{}:{}", r.kind, r.name);
            if local.as_deref() == Some(id.as_str()) {
                1
            } else if remote.as_deref() == Some(id.as_str()) {
                2
            } else if r.color.is_some() {
                4
            } else {
                99
            }
        };
        refs.sort_by_key(order);
        rows.push(Row {
            sha: commit.sha.clone(),
            parents,
            head: commit.head,
            input,
            output,
            refs,
        });
    }
    rows
}

/// MonoCode `historyItemGraph`: one row's paths and its node.
pub fn row_graph(row: &Row) -> RowGraph {
    let (w, h, r) = (SWIMLANE_WIDTH, SWIMLANE_HEIGHT, SWIMLANE_CURVE_RADIUS);
    let input_index = row.input.iter().position(|n| n.id == row.sha);
    let circle = input_index.unwrap_or(row.input.len());
    let color = row
        .output
        .get(circle)
        .or(row.input.get(circle))
        .map_or(REF_COLOR, |l| l.color);
    let mut paths = Vec::new();
    let mut out = 0usize;
    for (index, lane) in row.input.iter().enumerate() {
        let x = |i: usize| w * (i as f32 + 1.0);
        if lane.id == row.sha {
            if index != circle {
                paths.push(Path {
                    cmds: vec![
                        Cmd::Move(x(index), 0.0),
                        Cmd::Arc {
                            r: w,
                            sweep: true,
                            x: w * index as f32,
                            y: w,
                        },
                        Cmd::Line(x(circle), w),
                    ],
                    color: lane.color,
                });
            } else {
                out += 1;
            }
        } else if out < row.output.len() && lane.id == row.output[out].id {
            let cmds = if index == out {
                vec![Cmd::Move(x(index), 0.0), Cmd::Line(x(index), h)]
            } else {
                vec![
                    Cmd::Move(x(index), 0.0),
                    Cmd::Line(x(index), 6.0),
                    Cmd::Arc {
                        r,
                        sweep: true,
                        x: x(index) - r,
                        y: h / 2.0,
                    },
                    Cmd::Line(x(out) + r, h / 2.0),
                    Cmd::Arc {
                        r,
                        sweep: false,
                        x: x(out),
                        y: h / 2.0 + r,
                    },
                    Cmd::Line(x(out), h),
                ]
            };
            paths.push(Path {
                cmds,
                color: lane.color,
            });
            out += 1;
        }
    }
    for parent in row.parents.iter().skip(1) {
        let Some(at) = row.output.iter().rposition(|n| &n.id == parent) else {
            continue;
        };
        paths.push(Path {
            cmds: vec![
                Cmd::Move(w * at as f32, h / 2.0),
                Cmd::Arc {
                    r: w,
                    sweep: true,
                    x: w * (at as f32 + 1.0),
                    y: h,
                },
                Cmd::Move(w * at as f32, h / 2.0),
                Cmd::Line(w * (circle as f32 + 1.0), h / 2.0),
            ],
            color: row.output[at].color,
        });
    }
    let cx = w * (circle as f32 + 1.0);
    if let Some(i) = input_index {
        paths.push(Path {
            cmds: vec![Cmd::Move(cx, 0.0), Cmd::Line(cx, h / 2.0)],
            color: row.input[i].color,
        });
    }
    if !row.parents.is_empty() {
        paths.push(Path {
            cmds: vec![Cmd::Move(cx, h / 2.0), Cmd::Line(cx, h)],
            color,
        });
    }
    let lanes = row.input.len().max(row.output.len()).max(1);
    RowGraph {
        width: w * (lanes as f32 + 1.0),
        paths,
        cx,
        cy: w,
        node: if row.head {
            Node::Head
        } else if row.parents.len() > 1 {
            Node::Merge
        } else {
            Node::Commit
        },
        color,
    }
}

/// The node's outer radius (MonoCode's circles).
pub fn node_radius(node: Node) -> f32 {
    match node {
        Node::Head => CIRCLE_RADIUS + 3.0,
        Node::Merge => CIRCLE_RADIUS + 2.0,
        Node::Commit => CIRCLE_RADIUS + 1.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commit(sha: &str, parents: &[&str], refs: &[(&str, &str)], head: bool) -> HistoryCommit {
        HistoryCommit {
            sha: sha.into(),
            short_sha: sha.into(),
            parents: parents.iter().map(|p| p.to_string()).collect(),
            author: String::new(),
            timestamp: 0,
            subject: String::new(),
            refs: refs
                .iter()
                .map(|(n, k)| HistoryRef {
                    name: n.to_string(),
                    kind: k.to_string(),
                })
                .collect(),
            head,
        }
    }

    #[test]
    fn a_line_stays_in_one_lane_in_the_branch_colour() {
        let commits = [
            commit("c", &["b"], &[("main", "local"), ("origin/main", "remote")], true),
            commit("b", &["a"], &[], false),
            commit("a", &[], &[], false),
        ];
        let rows = layout(&commits);
        assert_eq!(rows[0].output, [Swimlane { id: "b".into(), color: REF_COLOR }]);
        assert_eq!(rows[1].output[0].color, REF_COLOR);
        assert!(rows[2].output.is_empty());
        assert_eq!(rows[0].refs[0].name, "main");
        assert_eq!(rows[0].refs[1].color, Some(REMOTE_REF_COLOR));
        let graph = row_graph(&rows[1]);
        assert_eq!(graph.cx, SWIMLANE_WIDTH);
        assert_eq!(graph.node, Node::Commit);
        assert_eq!(row_graph(&rows[0]).node, Node::Head);
    }

    #[test]
    fn a_merge_opens_a_second_lane() {
        let commits = [
            commit("m", &["a", "f"], &[], true),
            commit("f", &["a"], &[], false),
            commit("a", &[], &[], false),
        ];
        let rows = layout(&commits);
        assert_eq!(rows[0].output.len(), 2);
        assert_eq!(rows[0].output[0].color, LANE_COLORS[0]);
        assert_eq!(rows[0].output[1].color, LANE_COLORS[1]);
        let merge = row_graph(&rows[0]);
        assert_eq!(merge.width, SWIMLANE_WIDTH * 3.0);
        // The feature commit sits in the second lane.
        assert_eq!(row_graph(&rows[1]).cx, SWIMLANE_WIDTH * 2.0);
        assert_eq!(rows[2].input.len(), 2, "both lanes meet at the root");
    }
}
