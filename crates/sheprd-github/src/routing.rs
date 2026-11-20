//! Which signed-in account reads a repository: on the repository's server
//! only, an additional account when the owner is its login, one of its
//! organizations, or an owner its config lists; otherwise the main sign-in.
//! Workers learn a checkout's repository from its remote, so they carry
//! every token and choose once they know it.

use super::{Profile, Server};
use secrecy::SecretString;
use std::sync::Arc;

/// A token and the server its account signed in to. Requests made with it
/// go to that server's API only.
#[derive(Clone)]
pub struct ServerToken {
    pub server: Server,
    pub token: Arc<SecretString>,
}

impl ServerToken {
    /// A GitHub.com token.
    pub fn github(token: Arc<SecretString>) -> Self {
        Self {
            server: Server::GitHub,
            token,
        }
    }
}

/// One account's token and the owners it reads; the main account's list is
/// empty because it reads every owner on its server no other account claims.
struct Route {
    login: String,
    token: ServerToken,
    owners: Vec<String>,
}

impl Route {
    fn covers(&self, owner: &str) -> bool {
        self.owners
            .iter()
            .any(|covered| covered.eq_ignore_ascii_case(owner))
    }
}

/// The tokens a window reads GitHub with. Cheap to clone; the first route is
/// the main sign-in.
#[derive(Clone)]
pub struct Tokens(Arc<[Route]>);

impl Tokens {
    /// `main` for every owner, except those an additional account covers:
    /// its login, its organizations, and `owners` from its config. The first
    /// listed account covering an owner wins.
    pub fn new<'a>(
        main: &Profile,
        additional: impl IntoIterator<Item = (&'a Profile, &'a [String])>,
    ) -> Self {
        let main = Route {
            login: main.login.clone(),
            token: ServerToken::github(main.token.clone()),
            owners: Vec::new(),
        };
        let routes =
            std::iter::once(main).chain(additional.into_iter().map(|(profile, owners)| {
                Route {
                    login: profile.login.clone(),
                    token: ServerToken::github(profile.token.clone()),
                    owners: std::iter::once(&profile.login)
                        .chain(profile.organizations.iter().flatten())
                        .chain(owners)
                        .cloned()
                        .collect(),
                }
            }));
        Self(routes.collect())
    }

    fn route(&self, server: &Server, owner: &str) -> Option<&Route> {
        let serves = |route: &&Route| route.token.server.serves(server);
        self.0
            .iter()
            .skip(1)
            .filter(serves)
            .find(|route| route.covers(owner))
            .or_else(|| self.0.first().filter(serves))
    }

    /// The token that reads repositories owned by `owner` on `server`, the
    /// server a remote names, when an account there does.
    pub fn for_owner(&self, server: &Server, owner: &str) -> Option<&ServerToken> {
        self.route(server, owner).map(|route| &route.token)
    }

    /// The main sign-in's token.
    pub fn main(&self) -> &ServerToken {
        &self.0[0].token
    }

    /// The login of the account that reads repositories owned by `owner` on
    /// `server`.
    pub fn login_for(&self, server: &Server, owner: &str) -> Option<&str> {
        self.route(server, owner).map(|route| route.login.as_str())
    }

    /// The same accounts, owners, and token allocations. A renewed token is a
    /// new allocation, which is how caches notice it, never by its text.
    pub fn same(&self, other: &Self) -> bool {
        self.0.len() == other.0.len()
            && self.0.iter().zip(other.0.iter()).all(|(this, that)| {
                Arc::ptr_eq(&this.token.token, &that.token.token)
                    && this.token.server == that.token.server
                    && this.login == that.login
                    && this.owners == that.owners
            })
    }

    /// Every login, main first, each additional one with the owners it
    /// reads: names the account set answers belong to. Moving an owner to
    /// another account is a new set, so no answer or hidden repository the
    /// old account recorded is shown for the new one.
    pub fn key(&self) -> String {
        let routes: Vec<String> = self
            .0
            .iter()
            .map(|route| match route.owners.as_slice() {
                [] => route.login.clone(),
                owners => format!("{}({})", route.login, owners.join(" ")),
            })
            .collect();
        routes.join(",")
    }

    /// Every account's login, main first.
    pub fn logins(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(|route| route.login.as_str())
    }
}

/// One GitHub.com token read as every owner, as before additional accounts.
impl From<Arc<SecretString>> for Tokens {
    fn from(token: Arc<SecretString>) -> Self {
        Self(Arc::new([Route {
            login: String::new(),
            token: ServerToken::github(token),
            owners: Vec::new(),
        }]))
    }
}

#[cfg(test)]
mod tests;
