//! A* pathfinding on the terrain navmesh grid.
//!
//! Finds paths that respect walkability, movement costs, and terrain features.
//! Returns waypoint lists in world coordinates.

use std::collections::BinaryHeap;
use std::cmp::Ordering;

use crate::terrain::{TerrainGrid, GRID_RESOLUTION};
use crate::types::Vector3;

/// Maximum number of cells to explore before giving up.
const MAX_SEARCH_ITERATIONS: usize = 8000;

/// A node in the A* open set.
#[derive(Debug)]
struct AStarNode {
    col: usize,
    row: usize,
    /// Cost from start to this node.
    g_cost: f32,
    /// Estimated total cost (g + heuristic).
    f_cost: f32,
}

impl PartialEq for AStarNode {
    fn eq(&self, other: &Self) -> bool {
        self.f_cost == other.f_cost
    }
}

impl Eq for AStarNode {}

impl PartialOrd for AStarNode {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for AStarNode {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reverse ordering for min-heap (BinaryHeap is max-heap by default)
        other
            .f_cost
            .partial_cmp(&self.f_cost)
            .unwrap_or(Ordering::Equal)
    }
}

/// Find a path from `from` to `to` on the terrain grid.
///
/// Returns a list of world-space waypoints, or None if no path exists.
/// The path is smoothed to remove unnecessary intermediate waypoints.
pub fn find_path(grid: &TerrainGrid, from: Vector3, to: Vector3) -> Option<Vec<Vector3>> {
    let (start_col, start_row) = grid.world_to_cell(from.x, from.z)?;
    let (goal_col, goal_row) = grid.world_to_cell(to.x, to.z)?;

    // If start or goal is blocked, bail
    if grid.get_cell_walkability(start_col, start_row).is_blocked() {
        return None;
    }
    if grid.get_cell_walkability(goal_col, goal_row).is_blocked() {
        return None;
    }

    // Trivial case: already there
    if start_col == goal_col && start_row == goal_row {
        return Some(vec![to]);
    }

    // A* search
    let total_cells = GRID_RESOLUTION * GRID_RESOLUTION;
    let mut g_costs = vec![f32::INFINITY; total_cells];
    let mut came_from: Vec<Option<usize>> = vec![None; total_cells];
    let mut open = BinaryHeap::new();

    let start_idx = start_row * GRID_RESOLUTION + start_col;
    g_costs[start_idx] = 0.0;

    open.push(AStarNode {
        col: start_col,
        row: start_row,
        g_cost: 0.0,
        f_cost: heuristic(start_col, start_row, goal_col, goal_row),
    });

    let mut iterations = 0;

    while let Some(current) = open.pop() {
        iterations += 1;
        if iterations > MAX_SEARCH_ITERATIONS {
            return None; // Search limit reached
        }

        let cur_idx = current.row * GRID_RESOLUTION + current.col;

        // Skip if we've already found a better path to this cell
        if current.g_cost > g_costs[cur_idx] {
            continue;
        }

        // Reached goal
        if current.col == goal_col && current.row == goal_row {
            let cell_path = reconstruct_path(&came_from, cur_idx, start_idx);
            let world_path = cell_path_to_world(grid, &cell_path, to);
            return Some(smooth_path(grid, &world_path));
        }

        // Explore 8-connected neighbors
        for &(dc, dr) in &NEIGHBORS {
            let nc = current.col as isize + dc;
            let nr = current.row as isize + dr;

            if nc < 0 || nr < 0 || nc >= GRID_RESOLUTION as isize || nr >= GRID_RESOLUTION as isize
            {
                continue;
            }

            let nc = nc as usize;
            let nr = nr as usize;
            let n_idx = nr * GRID_RESOLUTION + nc;

            // Skip blocked cells
            if grid.get_cell_walkability(nc, nr).is_blocked() {
                continue;
            }

            // Diagonal movement cost: sqrt(2) for diagonal, 1 for cardinal
            let step_dist = if dc.abs() + dr.abs() == 2 {
                1.414
            } else {
                1.0
            };

            let move_cost = grid.get_cell_cost(nc, nr);
            let tentative_g = g_costs[cur_idx] + step_dist * move_cost;

            if tentative_g < g_costs[n_idx] {
                g_costs[n_idx] = tentative_g;
                came_from[n_idx] = Some(cur_idx);

                open.push(AStarNode {
                    col: nc,
                    row: nr,
                    g_cost: tentative_g,
                    f_cost: tentative_g + heuristic(nc, nr, goal_col, goal_row),
                });
            }
        }
    }

    None // No path found
}

/// 8-directional neighbor offsets.
const NEIGHBORS: [(isize, isize); 8] = [
    (-1, -1), (0, -1), (1, -1),
    (-1,  0),          (1,  0),
    (-1,  1), (0,  1), (1,  1),
];

/// Octile distance heuristic for 8-connected grid.
fn heuristic(col: usize, row: usize, goal_col: usize, goal_row: usize) -> f32 {
    let dx = (col as f32 - goal_col as f32).abs();
    let dy = (row as f32 - goal_row as f32).abs();
    let (min, max) = if dx < dy { (dx, dy) } else { (dy, dx) };
    // Octile distance
    max + (1.414 - 1.0) * min
}

/// Reconstruct cell path from came_from map.
fn reconstruct_path(
    came_from: &[Option<usize>],
    goal_idx: usize,
    start_idx: usize,
) -> Vec<(usize, usize)> {
    let mut path = Vec::new();
    let mut current = goal_idx;

    while current != start_idx {
        let row = current / GRID_RESOLUTION;
        let col = current % GRID_RESOLUTION;
        path.push((col, row));

        match came_from[current] {
            Some(prev) => current = prev,
            None => break,
        }
    }

    path.reverse();
    path
}

/// Convert cell indices to world-space waypoints.
fn cell_path_to_world(
    grid: &TerrainGrid,
    cell_path: &[(usize, usize)],
    final_target: Vector3,
) -> Vec<Vector3> {
    let mut world_path: Vec<Vector3> = cell_path
        .iter()
        .map(|&(col, row)| grid.cell_to_world(col, row))
        .collect();

    // Replace last waypoint with exact destination
    if let Some(last) = world_path.last_mut() {
        last.x = final_target.x;
        last.z = final_target.z;
        last.y = grid.get_elevation(final_target.x, final_target.z) as f64;
    }

    world_path
}

/// Remove unnecessary intermediate waypoints where straight-line movement is valid.
fn smooth_path(grid: &TerrainGrid, path: &[Vector3]) -> Vec<Vector3> {
    if path.len() <= 2 {
        return path.to_vec();
    }

    let mut smoothed = Vec::with_capacity(path.len());
    smoothed.push(path[0]);

    let mut anchor = 0;

    while anchor < path.len() - 1 {
        let mut furthest = anchor + 1;

        // Try to skip waypoints while maintaining line-of-sight
        for candidate in (anchor + 2)..path.len() {
            if line_walkable(grid, &path[anchor], &path[candidate]) {
                furthest = candidate;
            } else {
                break;
            }
        }

        smoothed.push(path[furthest]);
        anchor = furthest;
    }

    smoothed
}

/// Check if a straight line between two world points is fully walkable.
fn line_walkable(grid: &TerrainGrid, from: &Vector3, to: &Vector3) -> bool {
    let dx = to.x - from.x;
    let dz = to.z - from.z;
    let dist = (dx * dx + dz * dz).sqrt();

    // Sample at half-cell intervals
    let steps = (dist / (grid.cell_size * 0.5)).ceil() as usize;
    if steps == 0 {
        return true;
    }

    for i in 1..steps {
        let t = i as f64 / steps as f64;
        let x = from.x + dx * t;
        let z = from.z + dz * t;
        if !grid.is_walkable(x, z) {
            return false;
        }
    }

    true
}
