use super::types::{BesogneIR, ContentId, ResolvedInput};
use crate::manifest::Phase;
use petgraph::graph::{DiGraph, NodeIndex};
use std::collections::HashMap;

/// Build the exec-phase DAG from resolved inputs
pub fn build_exec_dag(
    ir: &BesogneIR,
) -> Result<(DiGraph<ContentId, ()>, HashMap<ContentId, NodeIndex>), String> {
    let mut graph = DiGraph::new();
    let mut node_map: HashMap<ContentId, NodeIndex> = HashMap::new();

    // Add all exec-phase inputs as nodes
    let exec_inputs: Vec<&ResolvedInput> = ir
        .inputs
        .iter()
        .filter(|i| i.phase == Phase::Exec)
        .collect();

    for input in &exec_inputs {
        let idx = graph.add_node(input.id.clone());
        node_map.insert(input.id.clone(), idx);
    }

    // Add ordering edges (after → before)
    for input in &exec_inputs {
        if let Some(node_idx) = node_map.get(&input.id) {
            for after_id in &input.after {
                if let Some(after_idx) = node_map.get(after_id) {
                    graph.add_edge(*after_idx, *node_idx, ());
                } else {
                    return Err(format!(
                        "exec input {} has after: [{}] which is not an exec-phase input",
                        input.id, after_id
                    ));
                }
            }
        }
    }

    // Check for cycles
    if petgraph::algo::is_cyclic_directed(&graph) {
        return Err("circular ordering in exec DAG".into());
    }

    Ok((graph, node_map))
}

/// Compute parallel execution tiers from topological sort
pub fn compute_tiers(
    graph: &DiGraph<ContentId, ()>,
) -> Result<Vec<Vec<NodeIndex>>, String> {
    let topo = petgraph::algo::toposort(graph, None)
        .map_err(|_| "circular ordering detected".to_string())?;

    let mut depth: HashMap<NodeIndex, usize> = HashMap::new();
    let mut max_depth = 0;

    for &node in &topo {
        let d = graph
            .neighbors_directed(node, petgraph::Direction::Incoming)
            .map(|pred| depth.get(&pred).copied().unwrap_or(0) + 1)
            .max()
            .unwrap_or(0);
        depth.insert(node, d);
        max_depth = max_depth.max(d);
    }

    let mut tiers: Vec<Vec<NodeIndex>> = vec![vec![]; max_depth + 1];
    for (node, d) in &depth {
        tiers[*d].push(*node);
    }

    Ok(tiers)
}
