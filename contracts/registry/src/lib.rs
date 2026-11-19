//! SplitVault Registry — on-chain index of all deployed split contract instances.
//!
//! Maintains a searchable map from creator address to their list of deployed
//! split contract addresses so frontends can discover active invoices.

#![no_std]

use soroban_sdk::{contract, contractimpl, symbol_short, Address, Env, Symbol, Vec};

// ---------------------------------------------------------------------------
// Storage keys
// ---------------------------------------------------------------------------

fn registry_admin_key() -> Symbol {
    symbol_short!("reg_adm")
}

/// Global pause flag — instance storage.
fn registry_paused_key() -> Symbol {
    symbol_short!("reg_pause")
}

/// Per-creator list of registered contract addresses — persistent storage.
fn creator_contracts_key(creator: &Address) -> (Symbol, Address) {
    (symbol_short!("cr_ctrs"), creator.clone())
}

/// Per-creator count of registered contracts — persistent storage.
///
/// Kept in sync with [`creator_contracts_key`] so `get_contract_count` can
/// answer in O(1) without deserializing the full address vector (#105).
fn creator_contract_count_key(creator: &Address) -> (Symbol, Address) {
    (symbol_short!("cr_cnt"), creator.clone())
}

/// Per-(creator, contract) flag to detect duplicate registrations — persistent.
fn registration_key(creator: &Address, contract: &Address) -> (Symbol, Address, Address) {
    (symbol_short!("reg_chk"), creator.clone(), contract.clone())
}

/// Maximum contracts per creator to bound storage growth and prevent DoS.
const MAX_CONTRACTS_PER_CREATOR: u32 = 5_000;

// ---------------------------------------------------------------------------
// Storage rent / TTL constants
// ---------------------------------------------------------------------------

/// Minimum TTL threshold before an extension is triggered (~60 days at 5 s/ledger).
const REGISTRY_MIN_TTL: u32 = 518_400;

/// Maximum TTL to extend to (~1 year at 5 s/ledger).
const REGISTRY_MAX_TTL: u32 = 6_307_200;

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

/// Emitted when a contract is registered for a creator.
///
/// Topics: (registry, registered, creator)
/// Data: contract_address
pub fn contract_registered(env: &Env, creator: &Address, contract: &Address) {
    env.events().publish(
        (
            symbol_short!("registry"),
            symbol_short!("register"),
            creator.clone(),
        ),
        contract.clone(),
    );
}

/// Emitted when a contract is deregistered from a creator.
///
/// Topics: (registry, deregister, creator)
/// Data: contract_address
pub fn contract_deregistered(env: &Env, creator: &Address, contract: &Address) {
    env.events().publish(
        (
            symbol_short!("registry"),
            symbol_short!("deregist"),
            creator.clone(),
        ),
        contract.clone(),
    );
}

/// Emitted when admin privileges are transferred.
///
/// Topics: (registry, set_admin)
/// Data: new_admin
pub fn admin_changed(env: &Env, new_admin: &Address) {
    env.events().publish(
        (symbol_short!("registry"), symbol_short!("set_admin")),
        new_admin.clone(),
    );
}

/// Emitted when the registry is paused or unpaused.
///
/// Topics: (registry, paused)
/// Data: paused flag
pub fn pause_changed(env: &Env, paused: bool) {
    env.events().publish(
        (symbol_short!("registry"), symbol_short!("paused")),
        paused,
    );
}

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

#[contract]
pub struct Registry;

#[contractimpl]
impl Registry {
    /// Initialise the registry by recording its admin.
    /// Can only be called once.
    pub fn initialize(env: Env, admin: Address) {
        assert!(
            !env.storage().instance().has(&registry_admin_key()),
            "already initialized"
        );
        env.storage().instance().set(&registry_admin_key(), &admin);
    }

    /// Return the current registry admin.
    pub fn get_admin(env: Env) -> Address {
        env.storage()
            .instance()
            .get(&registry_admin_key())
            .expect("not initialized")
    }

    /// Transfer admin privileges to `new_admin`. Requires current admin auth.
    pub fn set_admin(env: Env, new_admin: Address) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&registry_admin_key())
            .expect("not initialized");
        admin.require_auth();
        env.storage().instance().set(&registry_admin_key(), &new_admin);
        admin_changed(&env, &new_admin);
    }

    /// Pause or unpause registrations. Requires admin auth.
    ///
    /// While paused, [`Registry::register`] is rejected; admin management
    /// functions remain available.
    pub fn pause(env: Env, paused: bool) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&registry_admin_key())
            .expect("not initialized");
        admin.require_auth();
        env.storage().instance().set(&registry_paused_key(), &paused);
        pause_changed(&env, paused);
    }

    /// Return whether registrations are currently paused.
    pub fn is_paused(env: Env) -> bool {
        env.storage()
            .instance()
            .get(&registry_paused_key())
            .unwrap_or(false)
    }

    /// Remove a contract from a creator's registry. Requires admin auth.
    ///
    /// Used to purge spam or invalid entries. No-op if not registered.
    pub fn deregister_contract(env: Env, creator: Address, contract: Address) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&registry_admin_key())
            .expect("not initialized");
        admin.require_auth();

        let contracts_key = creator_contracts_key(&creator);
        let registration_check_key = registration_key(&creator, &contract);

        if !env.storage().persistent().has(&registration_check_key) {
            return;
        }

        let contracts: Vec<Address> = env
            .storage()
            .persistent()
            .get(&contracts_key)
            .unwrap_or_else(|| Vec::new(&env));

        let mut remaining: Vec<Address> = Vec::new(&env);
        let mut i = 0;
        while i < contracts.len() {
            let entry = contracts.get_unchecked(i);
            if entry != contract {
                remaining.push_back(entry);
            }
            i += 1;
        }

        if remaining.is_empty() {
            // Final removal: drop the list entirely so no rent-bearing empty
            // entry lingers. A later registration reconstructs it fresh.
            env.storage().persistent().remove(&contracts_key);
        } else {
            env.storage().persistent().set(&contracts_key, &remaining);
            env.storage()
                .persistent()
                .extend_ttl(&contracts_key, REGISTRY_MIN_TTL, REGISTRY_MAX_TTL);
        }

        env.storage()
            .persistent()
            .remove(&registration_check_key);

        contract_deregistered(&env, &creator, &contract);
    }

    /// Deregister a decommissioned contract from the caller's registry.
    ///
    /// Requires the `creator` to authenticate. Removes `contract` from the
    /// creator's active contract list, clears the duplicate-registration
    /// flag, and emits a [`contract_deregistered`] discovery event.
    ///
    /// # Panics
    /// * `"not registered"` — the contract is not in the creator's list.
    pub fn deregister(env: Env, creator: Address, contract: Address) {
        creator.require_auth();

        let contracts_key = creator_contracts_key(&creator);
        let registration_check_key = registration_key(&creator, &contract);

        assert!(
            env.storage().persistent().has(&registration_check_key),
            "not registered"
        );

        let contracts: Vec<Address> = env
            .storage()
            .persistent()
            .get(&contracts_key)
            .unwrap_or_else(|| Vec::new(&env));

        let mut remaining: Vec<Address> = Vec::new(&env);
        let mut i = 0;
        while i < contracts.len() {
            let entry = contracts.get_unchecked(i);
            if entry != contract {
                remaining.push_back(entry);
            }
            i += 1;
        }

        if remaining.is_empty() {
            env.storage().persistent().remove(&contracts_key);
        } else {
            env.storage().persistent().set(&contracts_key, &remaining);
            env.storage()
                .persistent()
                .extend_ttl(&contracts_key, REGISTRY_MIN_TTL, REGISTRY_MAX_TTL);
        }

        env.storage()
            .persistent()
            .remove(&registration_check_key);

        contract_deregistered(&env, &creator, &contract);
    }

    /// Register a deployed split contract under the caller's address.
    ///
    /// Requires the `creator` to authenticate. Rejects non-contract (account)
    /// addresses so only deployed contracts become discoverable, then appends
    /// `contract` to the creator's list and emits a [`contract_registered`]
    /// discovery event.
    ///
    /// # Panics
    /// * `"registry paused"` — registrations are currently paused.
    /// * `"not a contract address"` — `contract` is an account, not a contract.
    /// * `"already registered"` — `contract` is already in the creator's list.
    /// * `"too many contracts"` — the per-creator cap has been reached.
    pub fn register(env: Env, creator: Address, contract: Address) {
        creator.require_auth();

        assert!(!Self::is_paused(env.clone()), "registry paused");

        // Reject account addresses: only deployed contracts may be indexed.
        assert!(
            contract.executable(),
            "not a contract address"
        );

        let contracts_key = creator_contracts_key(&creator);
        let registration_check_key = registration_key(&creator, &contract);

        assert!(
            !env.storage().persistent().has(&registration_check_key),
            "already registered"
        );

        let mut contracts: Vec<Address> = env
            .storage()
            .persistent()
            .get(&contracts_key)
            .unwrap_or_else(|| Vec::new(&env));

        assert!(
            contracts.len() < MAX_CONTRACTS_PER_CREATOR,
            "too many contracts"
        );

        contracts.push_back(contract.clone());
        env.storage().persistent().set(&contracts_key, &contracts);
        env.storage()
            .persistent()
            .extend_ttl(&contracts_key, REGISTRY_MIN_TTL, REGISTRY_MAX_TTL);

        env.storage()
            .persistent()
            .set(&registration_check_key, &true);
        env.storage().persistent().extend_ttl(
            &registration_check_key,
            REGISTRY_MIN_TTL,
            REGISTRY_MAX_TTL,
        );

        let count_key = creator_contract_count_key(&creator);
        let count: u32 = env.storage().persistent().get(&count_key).unwrap_or(0);
        env.storage().persistent().set(&count_key, &(count + 1));
        env.storage()
            .persistent()
            .extend_ttl(&count_key, REGISTRY_MIN_TTL, REGISTRY_MAX_TTL);

        contract_registered(&env, &creator, &contract);
    }

    /// Return all contracts registered by `creator`.
    pub fn get_contracts(env: Env, creator: Address) -> Vec<Address> {
        env.storage()
            .persistent()
            .get(&creator_contracts_key(&creator))
            .unwrap_or_else(|| Vec::new(&env))
    }

    /// Return the number of contracts registered by `creator`.
    pub fn get_contract_count(env: Env, creator: Address) -> u32 {
        env.storage()
            .persistent()
            .get(&creator_contract_count_key(&creator))
            .unwrap_or(0)
    }
}
