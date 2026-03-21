//! Graph Builder for Temporal Arbitrage
//!
//! This module builds the temporal market graph by parsing market titles
//! and establishing parent-child relationships based on timeframe nesting
//! and time containment.

use crate::bot::strategy::temporal_arbitrage::{
    PriceState, TemporalNode, Timeframe, VolatilityEstimator,
};
use chrono::{Timelike, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Temporal graph structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemporalGraph {
    /// All nodes in the graph
    pub nodes: HashMap<String, TemporalNode>,
    /// Root nodes (no parent)
    pub roots: Vec<String>,
    /// Current price state
    pub price_state: PriceState,
    /// Volatility estimator (not serialized due to VecDeque)
    #[serde(skip)]
    pub vol_estimator: VolatilityEstimator,
    /// Last update timestamp
    pub last_update: i64,
}

impl Default for TemporalGraph {
    fn default() -> Self {
        Self {
            nodes: HashMap::new(),
            roots: Vec::new(),
            price_state: PriceState::default(),
            vol_estimator: VolatilityEstimator::default(),
            last_update: 0,
        }
    }
}

impl TemporalGraph {
    /// Create a new temporal graph
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a node to the graph
    pub fn add_node(&mut self, node: TemporalNode) {
        let id = node.condition_id.clone();
        self.nodes.insert(id.clone(), node);
    }

    /// Get a node by ID
    pub fn get_node(&self, id: &str) -> Option<&TemporalNode> {
        self.nodes.get(id)
    }

    /// Get a mutable node by ID
    pub fn get_node_mut(&mut self, id: &str) -> Option<&mut TemporalNode> {
        self.nodes.get_mut(id)
    }

    /// Update price for a node
    pub fn update_node_price(&mut self, id: &str, yes_price: f64, no_price: f64) -> bool {
        if let Some(node) = self.nodes.get_mut(id) {
            node.update_price(yes_price, no_price);
            true
        } else {
            false
        }
    }

    /// Mark a node as resolved
    pub fn mark_resolved(&mut self, id: &str, outcome: bool, close_price: f64) -> bool {
        if let Some(node) = self.nodes.get_mut(id) {
            node.mark_resolved(outcome, close_price);
            true
        } else {
            false
        }
    }

    /// Get all active nodes
    pub fn active_nodes(&self, now: i64) -> Vec<&TemporalNode> {
        self.nodes
            .values()
            .filter(|node| node.is_active(now))
            .collect()
    }

    /// Get all resolved nodes
    pub fn resolved_nodes(&self, now: i64) -> Vec<&TemporalNode> {
        self.nodes
            .values()
            .filter(|node| node.is_resolved(now))
            .collect()
    }

    /// Get children of a node
    pub fn get_children(&self, id: &str) -> Vec<&TemporalNode> {
        if let Some(node) = self.nodes.get(id) {
            node.children
                .iter()
                .filter_map(|child_id| self.nodes.get(child_id))
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Get parent of a node
    pub fn get_parent(&self, id: &str) -> Option<&TemporalNode> {
        if let Some(node) = self.nodes.get(id) {
            if let Some(parent_id) = &node.parent {
                self.nodes.get(parent_id)
            } else {
                None
            }
        } else {
            None
        }
    }

    /// Update global price state
    pub fn update_price_state(&mut self, btc_price: f64, ts: i64) {
        self.price_state.update(btc_price, ts);
        self.vol_estimator.update(btc_price);
        self.last_update = ts;
    }

    /// Get graph statistics
    pub fn stats(&self, now: i64) -> GraphStats {
        let total = self.nodes.len();
        let active = self.active_nodes(now).len();
        let resolved = self.resolved_nodes(now).len();

        let by_timeframe = {
            let mut counts = HashMap::new();
            for node in self.nodes.values() {
                *counts.entry(node.timeframe).or_insert(0) += 1;
            }
            counts
        };

        GraphStats {
            total_nodes: total,
            active_nodes: active,
            resolved_nodes: resolved,
            roots: self.roots.len(),
            by_timeframe,
        }
    }
}

/// Graph statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphStats {
    pub total_nodes: usize,
    pub active_nodes: usize,
    pub resolved_nodes: usize,
    pub roots: usize,
    pub by_timeframe: HashMap<Timeframe, usize>,
}

/// Builder for temporal market graphs
pub struct TemporalGraphBuilder {
    /// Regex patterns for parsing market titles
    time_patterns: Vec<Regex>,
    /// Regex for parsing timestamps
    timestamp_regex: Option<Regex>,
}

impl Default for TemporalGraphBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl TemporalGraphBuilder {
    /// Create a new graph builder
    pub fn new() -> Self {
        Self {
            time_patterns: vec![
                Regex::new(r"(\d+)\s*(m|min|minute|minutes)").unwrap(),
                Regex::new(r"(\d+)\s*(h|hr|hour|hours)").unwrap(),
            ],
            timestamp_regex: Some(
                Regex::new(r"(\d+):(\d+)\s*(AM|PM|am|pm)\s*-\s*(\d+):(\d+)\s*(AM|PM|am|pm)")
                    .unwrap(),
            ),
        }
    }

    /// Parse timeframe from market title
    ///
    /// Examples:
    /// - "BTC Up/Down 5m: March 15, 1:00-1:05PM ET" -> Timeframe::M5
    /// - "BTC 15min 3:00-3:15PM" -> Timeframe::M15
    pub fn parse_timeframe(title: &str) -> Option<Timeframe> {
        let normalized = title.to_ascii_lowercase();

        // Try explicit patterns first
        if normalized.contains("5m")
            || normalized.contains("5 min")
            || normalized.contains("5min")
            || normalized.contains("5-minute")
        {
            return Some(Timeframe::M5);
        }

        if normalized.contains("15m")
            || normalized.contains("15 min")
            || normalized.contains("15min")
            || normalized.contains("15-minute")
        {
            return Some(Timeframe::M15);
        }

        if normalized.contains("1h")
            || normalized.contains("1 hour")
            || normalized.contains("1hour")
            || normalized.contains("1-hour")
            || normalized.contains("60m")
            || normalized.contains("60 min")
        {
            return Some(Timeframe::H1);
        }

        if normalized.contains("4h")
            || normalized.contains("4 hour")
            || normalized.contains("4hour")
            || normalized.contains("4-hour")
            || normalized.contains("240m")
            || normalized.contains("240 min")
        {
            return Some(Timeframe::H4);
        }

        None
    }

    /// Parse timestamp from market title
    ///
    /// Returns (start_time, end_time) as Unix timestamps
    pub fn parse_timestamp(text: &str, timeframe: Timeframe) -> Option<(i64, i64)> {
        let re =
            Regex::new(r"(\d+):(\d+)\s*(AM|PM|am|pm)\s*-\s*(\d+):(\d+)\s*(AM|PM|am|pm)").ok()?;

        let caps = re.captures(text)?;

        let start_hour: i64 = caps.get(1)?.as_str().parse().ok()?;
        let start_min: i64 = caps.get(2)?.as_str().parse().ok()?;
        let start_ampm = caps.get(3)?.as_str().to_ascii_lowercase();
        let end_hour: i64 = caps.get(4)?.as_str().parse().ok()?;
        let end_min: i64 = caps.get(5)?.as_str().parse().ok()?;
        let end_ampm = caps.get(6)?.as_str().to_ascii_lowercase();

        let start_hour = if start_ampm == "pm" && start_hour != 12 {
            start_hour + 12
        } else if start_ampm == "am" && start_hour == 12 {
            0
        } else {
            start_hour
        };

        let end_hour = if end_ampm == "pm" && end_hour != 12 {
            end_hour + 12
        } else if end_ampm == "am" && end_hour == 12 {
            0
        } else {
            end_hour
        };

        let now = chrono::Utc::now();
        let mut start_time = now
            .with_hour(start_hour as u32)?
            .with_minute(start_min as u32)?
            .with_second(0)?
            .timestamp();

        let mut end_time = now
            .with_hour(end_hour as u32)?
            .with_minute(end_min as u32)?
            .with_second(0)?
            .timestamp();

        // Handle cross-noon case (e.g., 11:30AM - 12:00PM)
        if end_time < start_time {
            // End time is on the next day or same afternoon
            if end_hour >= 12 && start_hour < 12 {
                // Same afternoon case (e.g., 11:30AM - 12:00PM)
                end_time = start_time + timeframe.duration_secs();
            } else {
                // Next day case
                end_time += 86400;
            }
        }

        // Validate duration matches expected timeframe
        let duration = end_time - start_time;
        let expected = timeframe.duration_secs();

        // Allow some tolerance for parsing issues
        if (duration - expected).abs() > 60 {
            // If mismatch, try to adjust end_time
            end_time = start_time + expected;
        }

        Some((start_time, end_time))
    }

    /// Extract strike price from market metadata
    ///
    /// The "price to beat" is typically in the description or metadata.
    /// This is a best-effort extraction.
    pub fn extract_strike_price(description: &str) -> Option<f64> {
        // Try to find a price pattern in the description
        let price_re =
            Regex::new(r"\$?([\d,]+(?:\.\d+)?)\s*(?:strike|price|beat|ref|target)").ok()?;

        if let Some(caps) = price_re.captures(description) {
            let num_str = caps.get(1)?.as_str().replace(",", "");
            if let Ok(price) = num_str.parse::<f64>() {
                // Sanity check: BTC prices are typically 5-6 figures
                if price >= 1000.0 && price <= 200000.0 {
                    return Some(price);
                }
            }
        }

        // Try to find any 5-6 digit number that looks like a price
        let general_re = Regex::new(r"\$?(\d{4,6})(?:\.\d+)?").ok()?;
        for caps in general_re.captures_iter(description) {
            if let Some(num_str) = caps.get(1) {
                if let Ok(price) = num_str.as_str().parse::<f64>() {
                    if price >= 1000.0 && price <= 200000.0 {
                        return Some(price);
                    }
                }
            }
        }

        None
    }

    /// Build a graph from a list of temporal nodes
    pub fn build_from_nodes(
        &mut self,
        nodes: Vec<TemporalNode>,
        btc_price: f64,
        now: i64,
    ) -> TemporalGraph {
        let mut graph = TemporalGraph::new();

        // Add all nodes
        for node in nodes {
            graph.add_node(node);
        }

        // Update price state
        graph.update_price_state(btc_price, now);

        // Build hierarchy
        self.build_hierarchy(&mut graph);

        // Identify roots
        graph.roots = graph
            .nodes
            .values()
            .filter(|node| node.parent.is_none())
            .map(|node| node.condition_id.clone())
            .collect();

        graph
    }

    /// FIX #1: Build hierarchy WITHOUT checking strike equality
    ///
    /// Time containment + timeframe nesting is sufficient.
    /// Child strikes reset, so we DON'T check strike equality.
    fn build_hierarchy(&self, graph: &mut TemporalGraph) {
        let mut node_ids: Vec<_> = graph.nodes.keys().cloned().collect();

        // Sort by timeframe (longest first) then by start time
        node_ids.sort_by(|a, b| {
            let node_a = &graph.nodes[a];
            let node_b = &graph.nodes[b];

            // Longer timeframes first
            let tf_cmp = (node_b.timeframe as i32).cmp(&(node_a.timeframe as i32));
            if tf_cmp != std::cmp::Ordering::Equal {
                return tf_cmp;
            }

            node_a.start_time.cmp(&node_b.start_time)
        });

        // Collect hierarchy relationships first, then apply mutations
        let mut relationships: Vec<(String, String)> = Vec::new();
        let mut strike_inferences: Vec<(String, f64)> = Vec::new();

        for id in &node_ids {
            let node = &graph.nodes[id];
            let node_timeframe = node.timeframe;
            let node_start = node.start_time;
            let node_end = node.end_time;
            let node_strike = node.strike_price;

            // Try to find a parent
            for potential_parent in &node_ids {
                if potential_parent == id {
                    continue;
                }

                let parent = &graph.nodes[potential_parent];

                // Parent must be longer timeframe
                if parent.timeframe as i32 <= node_timeframe as i32 {
                    continue;
                }

                // FIX #1: Time containment + timeframe nesting is sufficient
                // Child strikes reset, so we DON'T check strike equality
                if node_start >= parent.start_time
                    && node_end <= parent.end_time
                    && Self::is_valid_child_by_timeframe(node_timeframe, parent.timeframe)
                {
                    relationships.push((id.clone(), potential_parent.clone()));

                    // FIX #3: If parent has no strike, infer from first child
                    if parent.strike_price == 0.0 && node_strike > 0.0 {
                        strike_inferences.push((potential_parent.clone(), node_strike));
                    }

                    break; // Found a parent, stop looking
                }
            }
        }

        // Apply the collected relationships
        for (child_id, parent_id) in relationships {
            if let Some(child_node) = graph.nodes.get_mut(&child_id) {
                child_node.parent = Some(parent_id.clone());
            }
            if let Some(parent_node) = graph.nodes.get_mut(&parent_id) {
                parent_node.children.push(child_id);
            }
        }

        // Apply strike inferences
        for (parent_id, strike) in strike_inferences {
            if let Some(parent_node) = graph.nodes.get_mut(&parent_id) {
                if parent_node.strike_price == 0.0 {
                    parent_node.strike_price = strike;
                }
            }
        }
    }

    /// Check if this is a valid parent-child relationship by timeframe
    fn is_valid_child_by_timeframe(child: Timeframe, parent: Timeframe) -> bool {
        // Check if parent timeframe is exactly one level up
        match (child, parent) {
            (Timeframe::M5, Timeframe::M15) => true,
            (Timeframe::M5, Timeframe::H1) => true,
            (Timeframe::M5, Timeframe::H4) => true,
            (Timeframe::M15, Timeframe::H1) => true,
            (Timeframe::M15, Timeframe::H4) => true,
            (Timeframe::H1, Timeframe::H4) => true,
            _ => false,
        }
    }

    /// Build graph from market titles (for testing/simple cases)
    pub fn build_from_titles(
        &mut self,
        markets: Vec<(String, String)>, // (condition_id, title)
        btc_price: f64,
        now: i64,
    ) -> TemporalGraph {
        let nodes: Vec<_> = markets
            .into_iter()
            .filter_map(|(id, title)| {
                let timeframe = TemporalGraphBuilder::parse_timeframe(&title)?;
                let (start_time, end_time) = Self::parse_timestamp(&title, timeframe)?;

                // Try to extract strike from title
                let strike_price = Self::extract_strike_price(&title).unwrap_or(0.0);

                Some(TemporalNode {
                    condition_id: id,
                    timeframe,
                    strike_price,
                    start_time,
                    end_time,
                    parent: None,
                    children: Vec::new(),
                    yes_price: None,
                    no_price: None,
                    resolved_outcome: None,
                    close_price: None,
                })
            })
            .collect();

        self.build_from_nodes(nodes, btc_price, now)
    }

    /// Find chains of related markets
    pub fn find_chains(&self, graph: &TemporalGraph) -> Vec<Vec<String>> {
        let mut chains = Vec::new();
        let mut visited = HashSet::new();

        for root in &graph.roots {
            if visited.contains(root) {
                continue;
            }

            let chain = self.collect_chain(graph, root);
            for id in &chain {
                visited.insert(id.clone());
            }
            chains.push(chain);
        }

        chains
    }

    /// Collect all nodes in a chain
    fn collect_chain(&self, graph: &TemporalGraph, root_id: &str) -> Vec<String> {
        let mut chain = Vec::new();
        let mut to_visit = vec![root_id.to_string()];
        let mut visited = HashSet::new();

        while let Some(id) = to_visit.pop() {
            if visited.contains(&id) {
                continue;
            }
            visited.insert(id.clone());

            if let Some(node) = graph.nodes.get(&id) {
                chain.push(id.clone());
                for child_id in &node.children {
                    to_visit.push(child_id.clone());
                }
            }
        }

        chain
    }

    /// Validate graph structure
    pub fn validate(&self, graph: &TemporalGraph) -> Result<(), GraphValidationError> {
        // Check for cycles
        if let Some(cycle) = self.detect_cycle(graph) {
            return Err(GraphValidationError::Cycle(cycle));
        }

        // Check for orphan nodes (except roots)
        for (id, node) in &graph.nodes {
            if node.parent.is_none() && !graph.roots.contains(id) {
                return Err(GraphValidationError::Orphan(id.clone()));
            }
        }

        // Check parent-child consistency
        for (id, node) in &graph.nodes {
            if let Some(parent_id) = &node.parent {
                if let Some(parent) = graph.nodes.get(parent_id) {
                    if !parent.children.contains(id) {
                        return Err(GraphValidationError::InconsistentParent(id.clone()));
                    }
                } else {
                    return Err(GraphValidationError::MissingParent(
                        id.clone(),
                        parent_id.clone(),
                    ));
                }
            }
        }

        Ok(())
    }

    /// Detect cycles in the graph
    fn detect_cycle(&self, graph: &TemporalGraph) -> Option<Vec<String>> {
        let mut visited = HashSet::new();
        let mut rec_stack = HashSet::new();
        let mut path = Vec::new();

        for id in graph.nodes.keys() {
            if self.dfs_cycle(graph, id, &mut visited, &mut rec_stack, &mut path) {
                return Some(path.clone());
            }
        }

        None
    }

    fn dfs_cycle(
        &self,
        graph: &TemporalGraph,
        id: &str,
        visited: &mut HashSet<String>,
        rec_stack: &mut HashSet<String>,
        path: &mut Vec<String>,
    ) -> bool {
        visited.insert(id.to_string());
        rec_stack.insert(id.to_string());
        path.push(id.to_string());

        if let Some(node) = graph.nodes.get(id) {
            for child_id in &node.children {
                if !visited.contains(child_id) {
                    if self.dfs_cycle(graph, child_id, visited, rec_stack, path) {
                        return true;
                    }
                } else if rec_stack.contains(child_id) {
                    path.push(child_id.clone());
                    return true;
                }
            }
        }

        path.pop();
        rec_stack.remove(id);
        false
    }
}

/// Graph validation error
#[derive(Debug, Clone)]
pub enum GraphValidationError {
    /// Cycle detected
    Cycle(Vec<String>),
    /// Orphan node found
    Orphan(String),
    /// Inconsistent parent-child relationship
    InconsistentParent(String),
    /// Missing parent reference
    MissingParent(String, String),
}

impl std::fmt::Display for GraphValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GraphValidationError::Cycle(path) => {
                write!(f, "Cycle detected: {:?}", path)
            }
            GraphValidationError::Orphan(id) => {
                write!(f, "Orphan node: {}", id)
            }
            GraphValidationError::InconsistentParent(id) => {
                write!(f, "Inconsistent parent for: {}", id)
            }
            GraphValidationError::MissingParent(id, parent) => {
                write!(f, "Missing parent {} for node {}", parent, id)
            }
        }
    }
}

impl std::error::Error for GraphValidationError {}

/// Thread-safe graph for concurrent access
pub type SharedTemporalGraph = Arc<RwLock<TemporalGraph>>;

/// Create a new shared temporal graph
pub fn new_shared_graph() -> SharedTemporalGraph {
    Arc::new(RwLock::new(TemporalGraph::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_timeframe() {
        assert_eq!(
            TemporalGraphBuilder::parse_timeframe("BTC Up/Down 5m: March 15"),
            Some(Timeframe::M5)
        );
        assert_eq!(
            TemporalGraphBuilder::parse_timeframe("BTC 15min market"),
            Some(Timeframe::M15)
        );
        assert_eq!(
            TemporalGraphBuilder::parse_timeframe("1 hour prediction"),
            Some(Timeframe::H1)
        );
        assert_eq!(
            TemporalGraphBuilder::parse_timeframe("4h contract"),
            Some(Timeframe::H4)
        );
        assert_eq!(
            TemporalGraphBuilder::parse_timeframe("Unknown market"),
            None
        );
    }

    #[test]
    fn test_parse_timestamp() {
        let title = "BTC Up/Down 5m: March 15, 1:00PM-1:05PM ET";
        let result = TemporalGraphBuilder::parse_timestamp(title, Timeframe::M5);

        assert!(result.is_some());
        let (start, end) = result.unwrap();
        assert_eq!(end - start, 300); // 5 minutes
    }

    #[test]
    fn test_extract_strike_price() {
        let desc = "Price to beat: $71,500. Will BTC go higher?";
        let price = TemporalGraphBuilder::extract_strike_price(desc);

        assert_eq!(price, Some(71500.0));
    }

    #[test]
    fn test_graph_add_get_node() {
        let mut graph = TemporalGraph::new();
        let node = TemporalNode {
            condition_id: "test-1".to_string(),
            timeframe: Timeframe::M5,
            strike_price: 71000.0,
            start_time: 1000,
            end_time: 1300,
            parent: None,
            children: Vec::new(),
            yes_price: None,
            no_price: None,
            resolved_outcome: None,
            close_price: None,
        };

        graph.add_node(node);
        assert!(graph.get_node("test-1").is_some());
        assert!(graph.get_node("test-2").is_none());
    }

    #[test]
    fn test_graph_update_price() {
        let mut graph = TemporalGraph::new();
        let node = TemporalNode {
            condition_id: "test-1".to_string(),
            timeframe: Timeframe::M5,
            strike_price: 71000.0,
            start_time: 1000,
            end_time: 1300,
            parent: None,
            children: Vec::new(),
            yes_price: None,
            no_price: None,
            resolved_outcome: None,
            close_price: None,
        };

        graph.add_node(node);

        let updated = graph.update_node_price("test-1", 0.55, 0.45);
        assert!(updated);

        let node = graph.get_node("test-1").unwrap();
        assert_eq!(node.yes_price, Some(0.55));
        assert_eq!(node.no_price, Some(0.45));
    }

    #[test]
    fn test_graph_mark_resolved() {
        let mut graph = TemporalGraph::new();
        let node = TemporalNode {
            condition_id: "test-1".to_string(),
            timeframe: Timeframe::M5,
            strike_price: 71000.0,
            start_time: 1000,
            end_time: 1300,
            parent: None,
            children: Vec::new(),
            yes_price: None,
            no_price: None,
            resolved_outcome: None,
            close_price: None,
        };

        graph.add_node(node);

        let updated = graph.mark_resolved("test-1", true, 71100.0);
        assert!(updated);

        let node = graph.get_node("test-1").unwrap();
        assert_eq!(node.resolved_outcome, Some(true));
        assert_eq!(node.close_price, Some(71100.0));
    }

    #[test]
    fn test_graph_active_resolved_nodes() {
        let mut graph = TemporalGraph::new();

        // Active node
        graph.add_node(TemporalNode {
            condition_id: "active".to_string(),
            timeframe: Timeframe::M5,
            strike_price: 71000.0,
            start_time: 1000,
            end_time: 10000,
            parent: None,
            children: Vec::new(),
            yes_price: None,
            no_price: None,
            resolved_outcome: None,
            close_price: None,
        });

        // Resolved node
        graph.add_node(TemporalNode {
            condition_id: "resolved".to_string(),
            timeframe: Timeframe::M5,
            strike_price: 71000.0,
            start_time: 1000,
            end_time: 2000,
            parent: None,
            children: Vec::new(),
            yes_price: None,
            no_price: None,
            resolved_outcome: Some(true),
            close_price: Some(71100.0),
        });

        let active = graph.active_nodes(5000);
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].condition_id, "active");

        let resolved = graph.resolved_nodes(5000);
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].condition_id, "resolved");
    }

    #[test]
    fn test_build_hierarchy() {
        let mut builder = TemporalGraphBuilder::new();

        let mut graph = TemporalGraph::new();

        // Add 5min nodes
        for i in 0..3 {
            let start = 1000 + i * 300;
            graph.add_node(TemporalNode {
                condition_id: format!("5m-{}", i),
                timeframe: Timeframe::M5,
                strike_price: 71000.0 + i as f64 * 10.0,
                start_time: start,
                end_time: start + 300,
                parent: None,
                children: Vec::new(),
                yes_price: None,
                no_price: None,
                resolved_outcome: None,
                close_price: None,
            });
        }

        // Add 15min parent
        graph.add_node(TemporalNode {
            condition_id: "15m-0".to_string(),
            timeframe: Timeframe::M15,
            strike_price: 0.0, // Will be inferred
            start_time: 1000,
            end_time: 1900,
            parent: None,
            children: Vec::new(),
            yes_price: None,
            no_price: None,
            resolved_outcome: None,
            close_price: None,
        });

        builder.build_hierarchy(&mut graph);

        // Check that parent-child relationships were established
        let parent = graph.get_node("15m-0").unwrap();
        assert_eq!(parent.children.len(), 3);
        assert_eq!(parent.strike_price, 71000.0); // Inferred from first child

        for i in 0..3 {
            let child = graph.get_node(&format!("5m-{}", i)).unwrap();
            assert_eq!(child.parent, Some("15m-0".to_string()));
        }
    }

    #[test]
    fn test_is_contained() {
        let builder = TemporalGraphBuilder::new();

        let parent = TemporalNode {
            condition_id: "parent".to_string(),
            timeframe: Timeframe::M15,
            strike_price: 71000.0,
            start_time: 1000,
            end_time: 1900,
            parent: None,
            children: Vec::new(),
            yes_price: None,
            no_price: None,
            resolved_outcome: None,
            close_price: None,
        };

        let child_contained = TemporalNode {
            condition_id: "child1".to_string(),
            timeframe: Timeframe::M5,
            strike_price: 71000.0,
            start_time: 1000,
            end_time: 1300,
            parent: None,
            children: Vec::new(),
            yes_price: None,
            no_price: None,
            resolved_outcome: None,
            close_price: None,
        };

        let child_not_contained = TemporalNode {
            condition_id: "child2".to_string(),
            timeframe: Timeframe::M5,
            strike_price: 71000.0,
            start_time: 2000,
            end_time: 2300,
            parent: None,
            children: Vec::new(),
            yes_price: None,
            no_price: None,
            resolved_outcome: None,
            close_price: None,
        };

        assert!(builder.is_contained(&child_contained, &parent));
        assert!(!builder.is_contained(&child_not_contained, &parent));
    }

    #[test]
    fn test_graph_stats() {
        let mut graph = TemporalGraph::new();

        graph.add_node(TemporalNode {
            condition_id: "active".to_string(),
            timeframe: Timeframe::M5,
            strike_price: 71000.0,
            start_time: 1000,
            end_time: 10000,
            parent: None,
            children: Vec::new(),
            yes_price: None,
            no_price: None,
            resolved_outcome: None,
            close_price: None,
        });

        graph.add_node(TemporalNode {
            condition_id: "resolved".to_string(),
            timeframe: Timeframe::M5,
            strike_price: 71000.0,
            start_time: 1000,
            end_time: 2000,
            parent: None,
            children: Vec::new(),
            yes_price: None,
            no_price: None,
            resolved_outcome: Some(true),
            close_price: Some(71100.0),
        });

        let stats = graph.stats(5000);

        assert_eq!(stats.total_nodes, 2);
        assert_eq!(stats.active_nodes, 1);
        assert_eq!(stats.resolved_nodes, 1);
    }
}
