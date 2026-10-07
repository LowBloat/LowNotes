//! Observed-remove set for manual and assistant links. links.json is a
//! projection; this immutable history is the replicated authority.
use crate::links::{LinkEdge, LinkOrigin};
use anyhow::bail;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const RELATIVE_PATH: &str = ".lownotes/link-operations.json";
pub const MAX_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkChanges {
    version: u8,
    #[serde(default)]
    pub additions: BTreeMap<String, LinkEdge>,
    #[serde(default)]
    pub removals: BTreeMap<String, BTreeSet<String>>,
}

impl Default for LinkChanges {
    fn default() -> Self {
        Self {
            version: 1,
            additions: BTreeMap::new(),
            removals: BTreeMap::new(),
        }
    }
}

fn unique_id(prefix: &str) -> String {
    let random: [u8; 16] = rand::random();
    format!(
        "{prefix}-{}",
        random
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

fn legacy_id(edge: &LinkEdge) -> String {
    format!(
        "legacy-{}",
        blake3::hash(&serde_json::to_vec(edge).expect("link serialization")).to_hex()
    )
}

fn valid_id(id: &str) -> bool {
    let Some((prefix, value)) = id.split_once('-') else {
        return false;
    };
    matches!(prefix, "add" | "remove" | "legacy")
        && value.len() == if prefix == "legacy" { 64 } else { 32 }
        && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn valid_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 4096
        && !path.contains(['\\', ':', '\0'])
        && !path.starts_with('/')
        && path
            .split('/')
            .all(|part| !part.is_empty() && !part.starts_with('.'))
        && crate::vault::is_markdown(std::path::Path::new(path))
}

impl LinkChanges {
    pub fn decode(bytes: &[u8]) -> anyhow::Result<Self> {
        if bytes.len() > MAX_BYTES {
            bail!("link operation history exceeds transfer limit");
        }
        let history: Self = serde_json::from_slice(bytes)?;
        history.validate()?;
        Ok(history)
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        if self.version != 1
            || self.additions.iter().any(|(id, edge)| {
                !valid_id(id)
                    || id.starts_with("remove-")
                    || !valid_path(&edge.source)
                    || !valid_path(&edge.target)
                    || edge.origin == LinkOrigin::wikilink
                    || (id.starts_with("legacy-") && *id != legacy_id(edge))
            })
            || self.removals.iter().any(|(id, tags)| {
                !valid_id(id)
                    || !id.starts_with("remove-")
                    || tags.is_empty()
                    || tags
                        .iter()
                        .any(|tag| !valid_id(tag) || tag.starts_with("remove-"))
            })
        {
            bail!("invalid link operation history");
        }
        Ok(())
    }

    pub fn encode(&self) -> anyhow::Result<Vec<u8>> {
        self.validate()?;
        let bytes = serde_json::to_vec_pretty(self)?;
        if bytes.len() > MAX_BYTES {
            bail!("link operation history exceeds transfer limit");
        }
        Ok(bytes)
    }

    pub fn merged(&self, remote: &Self) -> anyhow::Result<Self> {
        let mut merged = self.clone();
        for (id, edge) in &remote.additions {
            if merged.additions.get(id).is_some_and(|local| local != edge) {
                bail!("a link addition identity was reused");
            }
            merged.additions.insert(id.clone(), edge.clone());
        }
        for (id, tags) in &remote.removals {
            if merged.removals.get(id).is_some_and(|local| local != tags) {
                bail!("a link removal identity was reused");
            }
            merged.removals.insert(id.clone(), tags.clone());
        }
        merged.encode()?;
        Ok(merged)
    }

    fn removed_tags(&self) -> BTreeSet<&str> {
        self.removals
            .values()
            .flat_map(|tags| tags.iter().map(String::as_str))
            .collect()
    }

    pub fn edges(&self) -> Vec<LinkEdge> {
        let removed = self.removed_tags();
        self.additions
            .iter()
            .filter(|(id, _)| !removed.contains(id.as_str()))
            .map(|(_, edge)| edge.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    pub fn add(&mut self, edge: LinkEdge) {
        if edge.origin == LinkOrigin::wikilink || self.edges().contains(&edge) {
            return;
        }
        self.additions.insert(unique_id("add"), edge);
    }

    /// Lists from older versions have no identities; deterministic tags make
    /// their migration and repeated receipt idempotent.
    pub fn import_legacy(&mut self, edges: &[LinkEdge]) {
        let active: BTreeSet<LinkEdge> = self.edges().into_iter().collect();
        for edge in edges {
            if edge.origin != LinkOrigin::wikilink && !active.contains(edge) {
                self.additions
                    .entry(legacy_id(edge))
                    .or_insert_with(|| edge.clone());
            }
        }
    }

    /// Only observed additions are removed; a genuinely concurrent new add survives.
    pub fn remove(&mut self, source: &str, target: &str) {
        let removed = self.removed_tags();
        let mut tags: BTreeSet<String> = self
            .additions
            .iter()
            .filter(|(id, edge)| {
                edge.source == source && edge.target == target && !removed.contains(id.as_str())
            })
            .map(|(id, _)| id.clone())
            .collect();
        // Old clients cannot distinguish a stale projection from a re-add.
        // Block stale legacy tags; current clients re-add using fresh IDs.
        for origin in [LinkOrigin::manual, LinkOrigin::agent] {
            tags.insert(legacy_id(&LinkEdge {
                source: source.into(),
                target: target.into(),
                origin,
            }));
        }
        if tags.iter().all(|tag| removed.contains(tag.as_str())) {
            return;
        }
        self.removals.insert(unique_id("remove"), tags);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn edge(target: &str, origin: LinkOrigin) -> LinkEdge {
        LinkEdge {
            source: "a.md".into(),
            target: target.into(),
            origin,
        }
    }

    #[test]
    fn three_offline_replicas_merge_in_any_order_and_replaying_is_idempotent() {
        let mut a = LinkChanges::default();
        let mut b = LinkChanges::default();
        let mut c = LinkChanges::default();
        a.add(edge("b.md", LinkOrigin::manual));
        b.add(edge("c.md", LinkOrigin::agent));
        c.add(edge("d.md", LinkOrigin::manual));
        let expected = a.merged(&b).unwrap().merged(&c).unwrap();
        assert_eq!(expected, c.merged(&a).unwrap().merged(&b).unwrap());
        assert_eq!(expected, b.merged(&c).unwrap().merged(&a).unwrap());
        assert_eq!(expected.edges().len(), 3);
        assert_eq!(expected, expected.merged(&a).unwrap().merged(&b).unwrap());
    }

    #[test]
    fn removed_additions_and_stale_legacy_lists_do_not_resurrect_links() {
        let original = edge("b.md", LinkOrigin::manual);
        let mut old = LinkChanges::default();
        old.import_legacy(std::slice::from_ref(&original));
        let mut a = old.clone();
        a.remove("a.md", "b.md");
        assert!(a.merged(&old).unwrap().edges().is_empty());
        a.import_legacy(std::slice::from_ref(&original));
        assert!(a.edges().is_empty());
        a.add(original);
        assert_eq!(a.edges().len(), 1);
        assert_eq!(a.merged(&old).unwrap().edges(), a.edges());
    }

    #[test]
    fn concurrent_add_and_remove_preserve_the_unobserved_origin() {
        let mut a = LinkChanges::default();
        a.add(edge("b.md", LinkOrigin::manual));
        let mut b = a.clone();
        a.remove("a.md", "b.md");
        b.add(edge("b.md", LinkOrigin::agent));
        let merged = a.merged(&b).unwrap();
        assert_eq!(merged.edges(), vec![edge("b.md", LinkOrigin::agent)]);
        assert_eq!(merged, b.merged(&a).unwrap());
        let mut early = a.clone();
        early.additions.clear();
        assert_eq!(early.merged(&b).unwrap().edges(), merged.edges());
    }

    #[test]
    fn immutable_identity_collisions_and_invalid_paths_are_rejected() {
        let mut a = LinkChanges::default();
        a.add(edge("b.md", LinkOrigin::manual));
        let mut malformed = a.clone();
        malformed.additions.values_mut().next().unwrap().target = "c.md".into();
        assert!(a.merged(&malformed).is_err());
        malformed.additions.values_mut().next().unwrap().target = "../outside.md".into();
        assert!(LinkChanges::decode(&serde_json::to_vec(&malformed).unwrap()).is_err());
        assert_eq!(LinkChanges::decode(&a.encode().unwrap()).unwrap(), a);
    }
}
