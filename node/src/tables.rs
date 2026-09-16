use crate::gossip::{HeardTable, TableAdvert};
use crate::mesh::OpenTable;
use n0_future::time::{Duration, Instant};
use std::collections::{BTreeMap, BTreeSet};

pub const FIRST_HAND_EXPIRY: Duration = Duration::from_secs(150);
pub const RELAY_EXPIRY_SECS: u64 = 300;
pub const RELAY_EXPIRY: Duration = Duration::from_secs(RELAY_EXPIRY_SECS);

#[derive(Debug, Clone)]
struct FirstHand {
    advert: TableAdvert,
    seen: Instant,
    heard_at: u64,
}

#[derive(Debug, Clone)]
struct Relayed {
    heard_at: u64,
    learned: Instant,
}

#[derive(Debug, Default)]
pub struct TableBook {
    first_hand: BTreeMap<String, FirstHand>,
    relayed: BTreeMap<(String, String), Relayed>,
    withdrawn: BTreeMap<String, u64>,
}

pub fn fresh(heard_at: u64, epoch: u64) -> bool {
    epoch.saturating_sub(heard_at) < RELAY_EXPIRY_SECS
}

impl TableBook {
    pub fn hear(
        &mut self,
        host: &str,
        advert: Option<&TableAdvert>,
        now: Instant,
        epoch: u64,
    ) -> bool {
        let before = self.visible(now, epoch);
        self.relayed
            .retain(|(relayed_host, _), _| relayed_host != host);
        match advert {
            Some(advert) => {
                self.withdrawn.remove(host);
                self.first_hand.insert(
                    host.to_string(),
                    FirstHand {
                        advert: advert.clone(),
                        seen: now,
                        heard_at: epoch,
                    },
                );
            }
            None => {
                if self.first_hand.remove(host).is_some() || before.iter().any(|(h, _)| h == host) {
                    self.withdrawn.insert(host.to_string(), epoch);
                }
            }
        }
        before != self.visible(now, epoch)
    }

    pub fn learn(
        &mut self,
        sender: &str,
        heard: &[HeardTable],
        self_id: &str,
        known: impl Fn(&str) -> bool,
        now: Instant,
        epoch: u64,
    ) -> bool {
        let before = self.visible(now, epoch);
        self.sweep(now, epoch);
        for entry in heard {
            let host = entry.host.as_str();
            if host == self_id || host == sender || !known(host) {
                continue;
            }
            if !fresh(entry.heard_at, epoch) {
                continue;
            }
            if self
                .withdrawn
                .get(host)
                .is_some_and(|since| entry.heard_at <= *since)
            {
                continue;
            }
            if let Some(direct) = self.first_hand.get(host) {
                if now.duration_since(direct.seen) < FIRST_HAND_EXPIRY {
                    continue;
                }
                self.first_hand.remove(host);
            }
            let key = (host.to_string(), entry.table.name.clone());
            let newer = self
                .relayed
                .get(&key)
                .is_none_or(|existing| entry.heard_at > existing.heard_at);
            if newer {
                self.relayed.insert(
                    key,
                    Relayed {
                        heard_at: entry.heard_at,
                        learned: now,
                    },
                );
            }
        }
        before != self.visible(now, epoch)
    }

    pub fn forget(&mut self, host: &str, now: Instant, epoch: u64) -> bool {
        let before = self.visible(now, epoch);
        self.first_hand.remove(host);
        self.relayed
            .retain(|(relayed_host, _), _| relayed_host != host);
        before != self.visible(now, epoch)
    }

    pub fn live(&self, now: Instant, epoch: u64) -> Vec<OpenTable> {
        self.entries(now, epoch)
            .into_iter()
            .map(|(host, name, _, relayed)| OpenTable {
                host,
                name,
                relayed,
            })
            .collect()
    }

    pub fn relay(&self, now: Instant, epoch: u64) -> Vec<HeardTable> {
        self.entries(now, epoch)
            .into_iter()
            .map(|(host, name, heard_at, _)| HeardTable {
                host,
                table: TableAdvert { name },
                heard_at,
            })
            .collect()
    }

    fn entries(&self, now: Instant, epoch: u64) -> Vec<(String, String, u64, bool)> {
        let mut out: Vec<(String, String, u64, bool)> = self
            .first_hand
            .iter()
            .filter(|(_, direct)| now.duration_since(direct.seen) < FIRST_HAND_EXPIRY)
            .map(|(host, direct)| {
                (
                    host.clone(),
                    direct.advert.name.clone(),
                    direct.heard_at,
                    false,
                )
            })
            .collect();
        let direct_hosts: BTreeSet<&String> = out.iter().map(|(host, _, _, _)| host).collect();
        let direct_hosts: BTreeSet<String> = direct_hosts.into_iter().cloned().collect();
        out.extend(
            self.relayed
                .iter()
                .filter(|((host, _), _)| !direct_hosts.contains(host))
                .filter(|(_, entry)| {
                    fresh(entry.heard_at, epoch) && now.duration_since(entry.learned) < RELAY_EXPIRY
                })
                .map(|((host, name), entry)| (host.clone(), name.clone(), entry.heard_at, true)),
        );
        out.sort();
        out
    }

    fn visible(&self, now: Instant, epoch: u64) -> BTreeSet<(String, String)> {
        self.entries(now, epoch)
            .into_iter()
            .map(|(host, name, _, _)| (host, name))
            .collect()
    }

    fn sweep(&mut self, now: Instant, epoch: u64) {
        self.relayed.retain(|_, entry| {
            fresh(entry.heard_at, epoch) && now.duration_since(entry.learned) < RELAY_EXPIRY
        });
        self.withdrawn.retain(|_, since| fresh(*since, epoch));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPOCH: u64 = 1_700_000_000;

    fn table(name: &str) -> TableAdvert {
        TableAdvert { name: name.into() }
    }

    fn heard(host: &str, name: &str, heard_at: u64) -> HeardTable {
        HeardTable {
            host: host.into(),
            table: table(name),
            heard_at,
        }
    }

    fn names(tables: &[OpenTable]) -> Vec<(String, String, bool)> {
        tables
            .iter()
            .map(|t| (t.host.clone(), t.name.clone(), t.relayed))
            .collect()
    }

    fn everyone(_: &str) -> bool {
        true
    }

    #[test]
    fn a_table_advert_appears_and_an_unchanged_refresh_is_quiet() {
        let mut book = TableBook::default();
        let now = Instant::now();
        assert!(book.hear("aa", Some(&table("kitchen")), now, EPOCH));
        assert!(!book.hear("aa", Some(&table("kitchen")), now, EPOCH + 5));
        assert!(book.hear("aa", Some(&table("porch")), now, EPOCH));
        assert_eq!(
            names(&book.live(now, EPOCH)),
            vec![("aa".to_string(), "porch".to_string(), false)]
        );
    }

    #[test]
    fn a_withdrawn_table_disappears_immediately() {
        let mut book = TableBook::default();
        let now = Instant::now();
        book.hear("aa", Some(&table("kitchen")), now, EPOCH);
        assert!(book.hear("aa", None, now, EPOCH));
        assert!(!book.hear("aa", None, now, EPOCH));
        assert!(book.live(now, EPOCH).is_empty());
    }

    #[test]
    fn a_silent_host_expires_and_a_refreshed_one_does_not() {
        let mut book = TableBook::default();
        let start = Instant::now();
        book.hear("aa", Some(&table("stale")), start, EPOCH);
        book.hear("bb", Some(&table("fresh")), start, EPOCH);
        let later = start + FIRST_HAND_EXPIRY;
        book.hear("bb", Some(&table("fresh")), later, EPOCH + 150);
        assert_eq!(
            names(&book.live(later, EPOCH + 150)),
            vec![("bb".to_string(), "fresh".to_string(), false)]
        );
    }

    #[test]
    fn a_relayed_table_is_learned_listed_and_relayed_onward_with_its_original_timestamp() {
        let mut book = TableBook::default();
        let now = Instant::now();
        let changed = book.learn(
            "relay",
            &[heard("host", "porch", EPOCH - 10)],
            "me",
            everyone,
            now,
            EPOCH,
        );
        assert!(changed);
        assert_eq!(
            names(&book.live(now, EPOCH)),
            vec![("host".to_string(), "porch".to_string(), true)]
        );
        assert_eq!(
            book.relay(now, EPOCH),
            vec![heard("host", "porch", EPOCH - 10)]
        );
        let again = book.learn(
            "relay",
            &[heard("host", "porch", EPOCH - 10)],
            "me",
            everyone,
            now,
            EPOCH + 1,
        );
        assert!(!again);
    }

    #[test]
    fn our_own_table_and_the_senders_own_table_are_never_learned_second_hand() {
        let mut book = TableBook::default();
        let now = Instant::now();
        let changed = book.learn(
            "relay",
            &[heard("me", "mine", EPOCH), heard("relay", "theirs", EPOCH)],
            "me",
            everyone,
            now,
            EPOCH,
        );
        assert!(!changed);
        assert!(book.live(now, EPOCH).is_empty());
        assert!(book.relay(now, EPOCH).is_empty());
    }

    #[test]
    fn an_unknown_host_is_not_learned() {
        let mut book = TableBook::default();
        let now = Instant::now();
        let changed = book.learn(
            "relay",
            &[heard("stranger", "porch", EPOCH)],
            "me",
            |id| id != "stranger",
            now,
            EPOCH,
        );
        assert!(!changed);
        assert!(book.live(now, EPOCH).is_empty());
    }

    #[test]
    fn a_relayed_table_expires_by_its_heard_at_and_by_when_we_learned_it() {
        let mut book = TableBook::default();
        let now = Instant::now();
        assert!(!book.learn(
            "relay",
            &[heard("old", "porch", EPOCH - RELAY_EXPIRY_SECS)],
            "me",
            everyone,
            now,
            EPOCH,
        ));
        assert!(book.learn(
            "relay",
            &[heard("host", "porch", EPOCH - 1)],
            "me",
            everyone,
            now,
            EPOCH,
        ));
        assert_eq!(book.live(now, EPOCH).len(), 1);
        assert!(book.live(now, EPOCH + RELAY_EXPIRY_SECS).is_empty());
        assert!(book.live(now + RELAY_EXPIRY, EPOCH + 1).is_empty());
        assert!(book.relay(now + RELAY_EXPIRY, EPOCH + 1).is_empty());
    }

    #[test]
    fn hearsay_dedupes_by_host_and_table_keeping_the_newest_timestamp() {
        let mut book = TableBook::default();
        let now = Instant::now();
        book.learn(
            "b",
            &[heard("host", "porch", EPOCH - 20)],
            "me",
            everyone,
            now,
            EPOCH,
        );
        book.learn(
            "c",
            &[heard("host", "porch", EPOCH - 5)],
            "me",
            everyone,
            now,
            EPOCH,
        );
        book.learn(
            "d",
            &[heard("host", "porch", EPOCH - 30)],
            "me",
            everyone,
            now,
            EPOCH,
        );
        assert_eq!(book.live(now, EPOCH).len(), 1);
        assert_eq!(
            book.relay(now, EPOCH),
            vec![heard("host", "porch", EPOCH - 5)]
        );
    }

    #[test]
    fn a_first_hand_advert_supersedes_hearsay_about_the_same_host() {
        let mut book = TableBook::default();
        let now = Instant::now();
        book.learn(
            "relay",
            &[heard("host", "old-name", EPOCH - 5)],
            "me",
            everyone,
            now,
            EPOCH,
        );
        assert!(book.hear("host", Some(&table("new-name")), now, EPOCH));
        assert_eq!(
            names(&book.live(now, EPOCH)),
            vec![("host".to_string(), "new-name".to_string(), false)]
        );
        assert!(!book.learn(
            "relay",
            &[heard("host", "old-name", EPOCH - 5)],
            "me",
            everyone,
            now,
            EPOCH,
        ));
        assert_eq!(book.live(now, EPOCH).len(), 1);
    }

    #[test]
    fn a_clean_withdrawal_outranks_stale_hearsay_but_not_a_reopened_table() {
        let mut book = TableBook::default();
        let now = Instant::now();
        book.hear("host", Some(&table("porch")), now, EPOCH);
        assert!(book.hear("host", None, now, EPOCH + 10));
        assert!(!book.learn(
            "relay",
            &[heard("host", "porch", EPOCH + 5)],
            "me",
            everyone,
            now,
            EPOCH + 12,
        ));
        assert!(book.live(now, EPOCH + 12).is_empty());
        assert!(book.learn(
            "relay",
            &[heard("host", "porch", EPOCH + 20)],
            "me",
            everyone,
            now,
            EPOCH + 25,
        ));
        assert_eq!(book.live(now, EPOCH + 25).len(), 1);
    }

    #[test]
    fn hearsay_replaces_a_first_hand_advert_that_went_silent() {
        let mut book = TableBook::default();
        let start = Instant::now();
        book.hear("host", Some(&table("porch")), start, EPOCH);
        let later = start + FIRST_HAND_EXPIRY;
        let epoch = EPOCH + 150;
        assert!(book.live(later, epoch).is_empty());
        assert!(book.learn(
            "relay",
            &[heard("host", "porch", epoch - 3)],
            "me",
            everyone,
            later,
            epoch,
        ));
        assert_eq!(
            names(&book.live(later, epoch)),
            vec![("host".to_string(), "porch".to_string(), true)]
        );
    }

    #[test]
    fn forgetting_a_peer_drops_its_tables_from_both_books() {
        let mut book = TableBook::default();
        let now = Instant::now();
        book.hear("direct", Some(&table("a")), now, EPOCH);
        book.learn(
            "relay",
            &[heard("far", "b", EPOCH)],
            "me",
            everyone,
            now,
            EPOCH,
        );
        assert_eq!(book.live(now, EPOCH).len(), 2);
        assert!(book.forget("direct", now, EPOCH));
        assert!(book.forget("far", now, EPOCH));
        assert!(!book.forget("far", now, EPOCH));
        assert!(book.live(now, EPOCH).is_empty());
    }
}
