#![cfg(test)]
#![allow(clippy::all)]

use super::*;
use soroban_sdk::{testutils::Address as _, Address, Env};

fn setup() -> (Env, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();

    let split_id = Address::generate(&env);
    let admin = Address::generate(&env);
    let dashboard_id = env.register(DashboardContract, ());

    let client = DashboardContractClient::new(&env, &dashboard_id);
    client.initialize(&admin, &split_id);

    (env, split_id, dashboard_id)
}

#[test]
fn test_initial_dashboard_empty() {
    let (env, _split_id, dashboard_id) = setup();
    let client = DashboardContractClient::new(&env, &dashboard_id);

    let creator = Address::generate(&env);
    let dash = client.get_creator_dashboard(&creator);

    assert_eq!(dash.invoice_count, 0);
    assert_eq!(dash.total_volume, 0);
    assert_eq!(dash.released_count, 0);
    assert_eq!(dash.refunded_count, 0);
    assert_eq!(dash.released_volume, 0);
}

#[test]
fn test_record_created_updates_stats() {
    let (env, split_id, dashboard_id) = setup();
    let client = DashboardContractClient::new(&env, &dashboard_id);

    let creator = Address::generate(&env);

    client.record_created(&split_id, &creator, &1000_i128);
    client.record_created(&split_id, &creator, &2000_i128);

    let dash = client.get_creator_dashboard(&creator);
    assert_eq!(dash.invoice_count, 2);
    assert_eq!(dash.total_volume, 3000);
    assert_eq!(dash.released_count, 0);
    assert_eq!(dash.refunded_count, 0);
    assert_eq!(dash.released_volume, 0);
}

#[test]
fn test_record_released_updates_stats() {
    let (env, split_id, dashboard_id) = setup();
    let client = DashboardContractClient::new(&env, &dashboard_id);

    let creator = Address::generate(&env);

    client.record_created(&split_id, &creator, &5000_i128);
    client.record_released(&split_id, &creator, &3000_i128);
    client.record_released(&split_id, &creator, &2000_i128);

    let dash = client.get_creator_dashboard(&creator);
    assert_eq!(dash.invoice_count, 1);
    assert_eq!(dash.total_volume, 5000);
    assert_eq!(dash.released_count, 2);
    assert_eq!(dash.refunded_count, 0);
    assert_eq!(dash.released_volume, 5000);
}

#[test]
fn test_record_refunded_updates_stats() {
    let (env, split_id, dashboard_id) = setup();
    let client = DashboardContractClient::new(&env, &dashboard_id);

    let creator = Address::generate(&env);

    client.record_created(&split_id, &creator, &1000_i128);
    client.record_refunded(&split_id, &creator, &1000_i128);

    let dash = client.get_creator_dashboard(&creator);
    assert_eq!(dash.invoice_count, 1);
    assert_eq!(dash.total_volume, 1000);
    assert_eq!(dash.released_count, 0);
    assert_eq!(dash.refunded_count, 1);
    assert_eq!(dash.released_volume, 0);
    assert_eq!(dash.refunded_volume, 1000);
}

#[test]
fn test_stats_are_per_creator() {
    let (env, split_id, dashboard_id) = setup();
    let client = DashboardContractClient::new(&env, &dashboard_id);

    let creator_a = Address::generate(&env);
    let creator_b = Address::generate(&env);

    client.record_created(&split_id, &creator_a, &100_i128);
    client.record_created(&split_id, &creator_a, &200_i128);
    client.record_created(&split_id, &creator_b, &500_i128);
    client.record_released(&split_id, &creator_a, &300_i128);
    client.record_refunded(&split_id, &creator_b, &500_i128);

    let dash_a = client.get_creator_dashboard(&creator_a);
    assert_eq!(dash_a.invoice_count, 2);
    assert_eq!(dash_a.total_volume, 300);
    assert_eq!(dash_a.released_count, 1);
    assert_eq!(dash_a.refunded_count, 0);
    assert_eq!(dash_a.released_volume, 300);

    let dash_b = client.get_creator_dashboard(&creator_b);
    assert_eq!(dash_b.invoice_count, 1);
    assert_eq!(dash_b.total_volume, 500);
    assert_eq!(dash_b.released_count, 0);
    assert_eq!(dash_b.refunded_count, 1);
    assert_eq!(dash_b.released_volume, 0);
}

#[test]
fn test_unauthorized_caller_rejected() {
    let env = Env::default();

    let split_id = Address::generate(&env);
    let admin = Address::generate(&env);
    let dashboard_id = env.register(DashboardContract, ());

    let client = DashboardContractClient::new(&env, &dashboard_id);
    client.initialize(&admin, &split_id);

    let creator = Address::generate(&env);

    // Call without mock_all_auths — require_auth() on the authorized address will fail
    // because the test caller is not the authorized split contract.
    assert!(client
        .try_record_created(&split_id, &creator, &100)
        .is_err());
    assert_eq!(
        client.get_creator_dashboard(&creator),
        CreatorDashboard::default()
    );
}

// ---------------------------------------------------------------------------
// #83 — Multi-instance split contract registry tests
// ---------------------------------------------------------------------------

#[contract]
struct DashboardReporter;

#[contractimpl]
impl DashboardReporter {
    pub fn create(env: Env, dashboard: Address, creator: Address, total: i128) {
        DashboardContractClient::new(&env, &dashboard).record_created(
            &env.current_contract_address(),
            &creator,
            &total,
        );
    }
}

#[test]
fn test_multiple_split_contracts_can_report() {
    let env = Env::default();
    let split_a = env.register(DashboardReporter, ());
    let split_b = env.register(DashboardReporter, ());
    let admin = Address::generate(&env);
    let dashboard_id = env.register(DashboardContract, ());
    let client = DashboardContractClient::new(&env, &dashboard_id);
    client.initialize(&admin, &split_a);

    let creator = Address::generate(&env);
    assert!(client
        .try_record_created(&split_a, &creator, &1000)
        .is_err());
    assert_eq!(
        client.get_creator_dashboard(&creator),
        CreatorDashboard::default()
    );
    let reporter_a = DashboardReporterClient::new(&env, &split_a);
    let reporter_b = DashboardReporterClient::new(&env, &split_b);
    reporter_a.create(&dashboard_id, &creator, &1000);
    let initial = client.get_creator_dashboard(&creator);
    assert_eq!(initial.invoice_count, 1);
    assert_eq!(initial.total_volume, 1000);
    assert!(reporter_b
        .try_create(&dashboard_id, &creator, &2000)
        .is_err());
    assert_eq!(client.get_creator_dashboard(&creator), initial);

    client.mock_all_auths().register_split_contract(&split_b);
    assert!(client.try_revoke_split_contract(&split_a).is_err());
    client.mock_all_auths().revoke_split_contract(&split_a);
    assert!(!client.is_authorized_split_contract(&split_a));
    assert!(client.is_authorized_split_contract(&split_b));
    assert!(reporter_a
        .try_create(&dashboard_id, &creator, &1000)
        .is_err());
    assert_eq!(client.get_creator_dashboard(&creator), initial);

    reporter_b.create(&dashboard_id, &creator, &2000);
    let dash = client.get_creator_dashboard(&creator);
    assert_eq!(dash.invoice_count, 2);
    assert_eq!(dash.total_volume, 3000);
}

/// Registering the same split contract twice is idempotent (#83).
#[test]
fn test_register_split_contract_idempotent() {
    let (env, split_a, dashboard_id) = setup();
    let client = DashboardContractClient::new(&env, &dashboard_id);

    client.register_split_contract(&split_a);
    assert!(client.is_authorized_split_contract(&split_a));
}

/// An unregistered split contract is not authorized (#83).
#[test]
fn test_unregistered_split_contract_not_authorized() {
    let (env, _split_a, dashboard_id) = setup();
    let client = DashboardContractClient::new(&env, &dashboard_id);

    let stranger = Address::generate(&env);
    assert!(!client.is_authorized_split_contract(&stranger));
}

// ---------------------------------------------------------------------------
// #84 — Admin rotation and authorized contract management tests
// ---------------------------------------------------------------------------

/// The admin can update the authorized contract, and the new address can report (#84).
#[test]
fn test_set_authorized_contract_as_admin() {
    let (env, _split_id, dashboard_id) = setup();
    let client = DashboardContractClient::new(&env, &dashboard_id);

    let new_split = Address::generate(&env);
    client.set_authorized_contract(&new_split);

    assert!(client.is_authorized_split_contract(&new_split));

    let creator = Address::generate(&env);
    client.record_created(&new_split, &creator, &1000_i128);

    let dash = client.get_creator_dashboard(&creator);
    assert_eq!(dash.invoice_count, 1);
    assert_eq!(dash.total_volume, 1000);
}

/// The admin can rotate the admin address (#84).
#[test]
fn test_set_admin_rotates_admin() {
    let (env, _split_id, dashboard_id) = setup();
    let client = DashboardContractClient::new(&env, &dashboard_id);

    let new_admin = Address::generate(&env);
    client.set_admin(&new_admin);

    // New admin can still perform administrative actions.
    let new_split = Address::generate(&env);
    client.set_authorized_contract(&new_split);
    assert!(client.is_authorized_split_contract(&new_split));
}

// ---------------------------------------------------------------------------
// #50 — Dashboard event emission tests
// ---------------------------------------------------------------------------

use soroban_sdk::testutils::Events as _;
use soroban_sdk::{symbol_short, TryFromVal, Val};

/// `record_created` must emit `(dash, created, creator)` (#50).
#[test]
fn test_record_created_emits_event() {
    let (env, split_id, dashboard_id) = setup();
    let client = DashboardContractClient::new(&env, &dashboard_id);
    let creator = Address::generate(&env);

    client.record_created(&split_id, &creator, &500_i128);

    assert!(
        find_dash_event_data(&env, symbol_short!("created")).is_some(),
        "record_created did not emit (dash, created) event",
    );
}

/// `record_released` must emit `(dash, released, creator)` (#50).
#[test]
fn test_record_released_emits_event() {
    let (env, split_id, dashboard_id) = setup();
    let client = DashboardContractClient::new(&env, &dashboard_id);
    let creator = Address::generate(&env);

    client.record_created(&split_id, &creator, &500_i128);
    client.record_released(&split_id, &creator, &500_i128);

    assert!(
        find_dash_event_data(&env, symbol_short!("released")).is_some(),
        "record_released did not emit (dash, released) event",
    );
}

/// `record_refunded` must emit `(dash, refunded, creator)` (#50).
#[test]
fn test_record_refunded_emits_event() {
    let (env, split_id, dashboard_id) = setup();
    let client = DashboardContractClient::new(&env, &dashboard_id);
    let creator = Address::generate(&env);

    client.record_refunded(&split_id, &creator, &100_i128);

    assert!(
        find_dash_event_data(&env, symbol_short!("refunded")).is_some(),
        "record_refunded did not emit (dash, refunded) event",
    );
}

// ---------------------------------------------------------------------------
// #107 — Dashboard event payload tests
// ---------------------------------------------------------------------------

/// Read the latest event for this action. The creator is the third topic.
fn find_dash_event_data(env: &Env, action: Symbol) -> Option<(Address, Val)> {
    let events = env.events().all();
    for event in events.events().iter().rev() {
        let soroban_sdk::xdr::ContractEventBody::V0(body) = &event.body;
        let topics: &[soroban_sdk::xdr::ScVal] = body.topics.as_ref();
        if let [category, event_action, creator] = topics {
            if Symbol::try_from_val(env, category).ok()? == symbol_short!("dash")
                && Symbol::try_from_val(env, event_action).ok()? == action
            {
                return Some((
                    Address::try_from_val(env, creator).ok()?,
                    Val::try_from_val(env, &body.data).ok()?,
                ));
            }
        }
    }
    None
}

/// `record_created` event data must carry creator, invoice_count, total_volume (#107).
#[test]
fn test_record_created_event_payload() {
    let (env, split_id, dashboard_id) = setup();
    let client = DashboardContractClient::new(&env, &dashboard_id);
    let creator = Address::generate(&env);

    client.record_created(&split_id, &creator, &1000_i128);
    client.record_created(&split_id, &creator, &2000_i128);

    let (ev_creator, data) =
        find_dash_event_data(&env, symbol_short!("created")).expect("missing created event");
    let (count, volume) = <(u64, i128)>::try_from_val(&env, &data).unwrap();
    assert_eq!(ev_creator, creator);
    assert_eq!(count, 2);
    assert_eq!(volume, 3000);
}

/// `record_released` event data must carry creator, released_count, released_volume (#107).
#[test]
fn test_record_released_event_payload() {
    let (env, split_id, dashboard_id) = setup();
    let client = DashboardContractClient::new(&env, &dashboard_id);
    let creator = Address::generate(&env);

    client.record_released(&split_id, &creator, &3000_i128);
    client.record_released(&split_id, &creator, &2000_i128);

    let (ev_creator, data) =
        find_dash_event_data(&env, symbol_short!("released")).expect("missing released event");
    let (count, volume) = <(u64, i128)>::try_from_val(&env, &data).unwrap();
    assert_eq!(ev_creator, creator);
    assert_eq!(count, 2);
    assert_eq!(volume, 5000);
}

/// `record_refunded` event data must carry creator and refunded_count (#107).
#[test]
fn test_record_refunded_event_payload() {
    let (env, split_id, dashboard_id) = setup();
    let client = DashboardContractClient::new(&env, &dashboard_id);
    let creator = Address::generate(&env);

    client.record_refunded(&split_id, &creator, &100);
    client.record_refunded(&split_id, &creator, &100);

    let (ev_creator, data) =
        find_dash_event_data(&env, symbol_short!("refunded")).expect("missing refunded event");
    assert_eq!(ev_creator, creator);
    assert_eq!(u64::try_from_val(&env, &data).unwrap(), 2);
}

#[test]
fn test_aggregate_ttl_is_refreshed_on_read_and_write() {
    use soroban_sdk::testutils::{storage::Persistent as _, Ledger as _};

    let (env, split_id, dashboard_id) = setup();
    let client = DashboardContractClient::new(&env, &dashboard_id);
    env.ledger().set_min_persistent_entry_ttl(10);

    let reader = Address::generate(&env);
    let writer = Address::generate(&env);
    let absent = Address::generate(&env);
    let existing = CreatorDashboard {
        invoice_count: 1,
        total_volume: 100,
        ..CreatorDashboard::default()
    };
    env.as_contract(&dashboard_id, || {
        let storage = env.storage().persistent();
        for creator in [&reader, &writer] {
            let key = creator_dashboard_key(creator);
            storage.set(&key, &existing);
            assert_eq!(storage.get_ttl(&key), 9);
        }
    });

    assert_eq!(client.get_creator_dashboard(&reader), existing);
    client.record_created(&split_id, &writer, &100);
    assert_eq!(
        client.get_creator_dashboard(&absent),
        CreatorDashboard::default()
    );
    env.as_contract(&dashboard_id, || {
        let storage = env.storage().persistent();
        let target = AGGREGATE_TTL_LEDGERS.min(env.storage().max_ttl());
        assert_eq!(storage.get_ttl(&creator_dashboard_key(&reader)), target);
        assert_eq!(storage.get_ttl(&creator_dashboard_key(&writer)), target);
        assert!(!storage.has(&creator_dashboard_key(&absent)));
    });
}
