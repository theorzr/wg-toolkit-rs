//! The players, standing in for their stored accounts: each name logging in has an
//! account id of its own, given at its first login and kept until the server stops.
//! They all have the same garage, see `AccountData`.

use std::collections::HashMap;


/// A client's player, standing in for its stored account.
#[derive(Debug, Clone)]
pub struct Player {
    /// Its account's id, its `databaseID`.
    pub id: u64,
    pub name: String,
    /// The revision of the account's data, which each update brings to the next.
    pub revision: i64,
}

/// The account ids given to names, see the module's documentation.
#[derive(Debug, Default)]
pub struct AccountIds {
    ids: HashMap<String, u64>,
}

impl AccountIds {

    /// A player of this name logging in, with its account's id, given now if it is its
    /// first login: from 1, as 0 is no account.
    pub fn player(&mut self, name: &str) -> Player {
        let next = self.ids.len() as u64 + 1;
        let id = *self.ids.entry(name.to_string()).or_insert(next);
        Player {
            id,
            name: name.to_string(),
            revision: 1,
        }
    }

}
