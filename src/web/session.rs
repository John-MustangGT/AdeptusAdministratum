//! Helpers for reading and writing the roster from the tower-sessions session.

use crate::roster::RosterList;
use tower_sessions::Session;

const ROSTER_KEY: &str = "roster";

pub async fn load_roster(session: &Session) -> RosterList {
    session
        .get::<RosterList>(ROSTER_KEY)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| RosterList::new("My Army"))
}

pub async fn save_roster(session: &Session, roster: &RosterList) {
    let _ = session.insert(ROSTER_KEY, roster).await;
}
