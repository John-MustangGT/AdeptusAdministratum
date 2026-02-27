//! Helpers for reading and writing the roster from the tower-sessions session.

use crate::roster::RosterList;
use tower_sessions::Session;

const ROSTER_KEY: &str = "roster";

pub async fn load_roster(session: &Session) -> RosterList {
    match session.get::<RosterList>(ROSTER_KEY).await {
        Ok(Some(roster)) => {
            tracing::debug!(
                entries = roster.entries.len(),
                first_options = ?roster.entries.first().map(|e| &e.selection.chosen_options),
                "load_roster: loaded from session"
            );
            roster
        }
        Ok(None) => {
            tracing::debug!("load_roster: no roster in session, returning default");
            RosterList::default()
        }
        Err(e) => {
            tracing::warn!("load_roster: session.get failed: {:?}", e);
            RosterList::default()
        }
    }
}

pub async fn save_roster(session: &Session, roster: &RosterList) {
    tracing::debug!(
        entries = roster.entries.len(),
        first_options = ?roster.entries.first().map(|e| &e.selection.chosen_options),
        "save_roster: inserting into session"
    );
    if let Err(e) = session.insert(ROSTER_KEY, roster).await {
        tracing::error!("save_roster: session.insert failed: {:?}", e);
        return;
    }
    // Explicitly flush to the backing store immediately so the next
    // request (e.g. an HTMX add that fires right after configure) sees
    // the updated chosen_options rather than a stale snapshot.
    if let Err(e) = session.save().await {
        tracing::error!("save_roster: session.save() failed: {:?}", e);
    } else {
        tracing::debug!("save_roster: session.save() completed successfully");
    }
}
