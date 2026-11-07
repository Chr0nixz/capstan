//! Recent history stores only opaque IDs. Display names are resolved from the unlocked vault.
use uuid::Uuid;

pub const MAX_RECENT_HOSTS: usize = 10;

#[derive(Debug, Clone, Copy)]
pub enum HistoryChange {
    Connected(Uuid),
    Remove(Uuid),
    Clear,
}

pub fn record(history: &mut Vec<Uuid>, id: Uuid) {
    if id.is_nil() {
        return;
    }
    history.retain(|existing| *existing != id);
    history.insert(0, id);
    history.truncate(MAX_RECENT_HOSTS);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_is_bounded_deduplicated_and_most_recent_first() {
        let ids = (1..=12).map(Uuid::from_u128).collect::<Vec<_>>();
        let mut history = Vec::new();
        for id in &ids {
            record(&mut history, *id);
        }
        assert_eq!(history, ids[2..].iter().rev().copied().collect::<Vec<_>>());
        record(&mut history, ids[5]);
        assert_eq!(history[0], ids[5]);
        assert_eq!(history.len(), MAX_RECENT_HOSTS);
        assert_eq!(history.iter().filter(|id| **id == ids[5]).count(), 1);
        record(&mut history, Uuid::nil());
        assert_eq!(history.len(), MAX_RECENT_HOSTS);
    }
}
