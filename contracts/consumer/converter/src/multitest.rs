mod virtual_staking_mock;

use cosmwasm_std::{coin, coins, Addr, Decimal, StdError, Uint128, Validator};
use cw_multi_test::{no_init, AppBuilder};
use mesh_apis::converter_api::sv::mt::ConverterApiProxy;
use mesh_apis::converter_api::RewardInfo;
use mesh_simple_price_feed::contract::sv::mt::CodeId as PriceFeedCodeId;
use mesh_simple_price_feed::contract::SimplePriceFeedContract;
use sylvia::multitest::{App, Proxy};
use virtual_staking_mock::sv::mt::CodeId as VirtualStakingCodeId;
use virtual_staking_mock::VirtualStakingMock;

use crate::contract::sv::mt::CodeId as ConverterCodeId;
use crate::contract::sv::mt::ConverterContractProxy;
use crate::contract::{custom, ConverterContract};
use crate::error::ContractError;
use crate::error::ContractError::Unauthorized;
use crate::multitest::virtual_staking_mock::sv::mt::VirtualStakingMockProxy;

const JUNO: &str = "ujuno";

pub type MtApp = cw_multi_test::BasicApp<custom::ConverterMsg, custom::ConverterQuery>;

struct SetupArgs<'a> {
    owner: &'a str,
    admin: &'a str,
    discount: Decimal,
    native_per_foreign: Decimal,
}

struct SetupResponse<'a> {
    price_feed: Proxy<'a, MtApp, SimplePriceFeedContract<'a>>,
    converter: Proxy<'a, MtApp, ConverterContract<'a>>,
    virtual_staking: Proxy<'a, MtApp, VirtualStakingMock<'a>>,
}

fn new_app() -> App<MtApp> {
    App::new(AppBuilder::new_custom().build(no_init))
}

fn setup<'a>(app: &'a App<MtApp>, args: SetupArgs<'a>) -> SetupResponse<'a> {
    let SetupArgs {
        owner,
        admin,
        discount,
        native_per_foreign,
    } = args;

    let price_feed_code = PriceFeedCodeId::store_code(app);
    let virtual_staking_code = VirtualStakingCodeId::store_code(app);
    let converter_code = ConverterCodeId::store_code(app);

    let price_feed = price_feed_code
        .instantiate(native_per_foreign, None)
        .with_label("Price Feed")
        .call(owner)
        .unwrap();

    let converter = converter_code
        .instantiate(
            price_feed.contract_addr.to_string(),
            discount,
            JUNO.to_owned(),
            virtual_staking_code.code_id(),
            true,
            Some(admin.to_owned()),
            50,
        )
        .with_label("Juno Converter")
        .with_admin(admin)
        .call(owner)
        .unwrap();

    let config = converter.config().unwrap();
    let virtual_staking_addr = Addr::unchecked(config.virtual_staking);
    // Ideally this should be initialized via `CodeId`.
    // Consider bellow approach
    //
    // let virtual_staking = virtual_staking_code.instantiate().call(owner).unwrap();
    let virtual_staking = Proxy::new(virtual_staking_addr, app);

    SetupResponse {
        price_feed,
        converter,
        virtual_staking,
    }
}

#[test]
fn instantiation() {
    let app = new_app();

    let owner = "sunny"; // Owner of the staking contract (i. e. the vault contract)
    let admin = "theman";
    let discount = Decimal::percent(40); // 1 OSMO worth of JUNO should give 0.6 OSMO of stake
    let native_per_foreign = Decimal::percent(50); // 1 JUNO is worth 0.5 OSMO

    let SetupResponse {
        price_feed,
        converter,
        virtual_staking,
    } = setup(
        &app,
        SetupArgs {
            owner,
            admin,
            discount,
            native_per_foreign,
        },
    );

    // check the config
    let config = converter.config().unwrap();
    assert_eq!(config.price_feed, price_feed.contract_addr.to_string());
    assert_eq!(config.adjustment, Decimal::percent(60));
    assert!(!config.virtual_staking.is_empty());

    // let's check we passed the admin here properly
    let vs_info = app
        .app()
        .wrap()
        .query_wasm_contract_info(&config.virtual_staking)
        .unwrap();
    assert_eq!(vs_info.admin, Some(admin.to_string()));

    // let's query virtual staking to find the owner
    let vs_config = virtual_staking.config().unwrap();
    assert_eq!(vs_config.converter, converter.contract_addr.to_string());
}

#[test]
fn ibc_stake_and_unstake() {
    let app = new_app();

    let owner = "sunny"; // Owner of the staking contract (i. e. the vault contract)
    let admin = "theman";
    let discount = Decimal::percent(40); // 1 OSMO worth of JUNO should give 0.6 OSMO of stake
    let native_per_foreign = Decimal::percent(50); // 1 JUNO is worth 0.5 OSMO

    let SetupResponse {
        price_feed: _,
        converter,
        virtual_staking,
    } = setup(
        &app,
        SetupArgs {
            owner,
            admin,
            discount,
            native_per_foreign,
        },
    );

    // no one is staked
    let val1 = "Val Kilmer";
    let val2 = "Valley Girl";
    assert!(virtual_staking.all_stake().unwrap().stakes.is_empty());
    assert_eq!(
        virtual_staking
            .stake(val1.to_string())
            .unwrap()
            .stake
            .u128(),
        0
    );
    assert_eq!(
        virtual_staking
            .stake(val2.to_string())
            .unwrap()
            .stake
            .u128(),
        0
    );

    // let's stake some
    converter
        .test_stake(owner.to_string(), val1.to_string(), coin(1000, JUNO))
        .call(owner)
        .unwrap();
    converter
        .test_stake(owner.to_string(), val2.to_string(), coin(4000, JUNO))
        .call(owner)
        .unwrap();

    // and unstake some
    converter
        .test_unstake(owner.to_string(), val2.to_string(), coin(2000, JUNO))
        .call(owner)
        .unwrap();

    // and check the stakes (1000 * 0.6 * 0.5 = 300) (2000 * 0.6 * 0.5 = 600)
    assert_eq!(
        virtual_staking
            .stake(val1.to_string())
            .unwrap()
            .stake
            .u128(),
        300
    );
    assert_eq!(
        virtual_staking
            .stake(val2.to_string())
            .unwrap()
            .stake
            .u128(),
        600
    );
    assert_eq!(
        virtual_staking.all_stake().unwrap().stakes,
        vec![
            (val1.to_string(), Uint128::new(300)),
            (val2.to_string(), Uint128::new(600)),
        ]
    );
}

#[test]
fn ibc_stake_and_burn() {
    let app = new_app();

    let owner = "sunny"; // Owner of the staking contract (i. e. the vault contract)
    let admin = "theman";
    let discount = Decimal::percent(40); // 1 OSMO worth of JUNO should give 0.6 OSMO of stake
    let native_per_foreign = Decimal::percent(50); // 1 JUNO is worth 0.5 OSMO

    let SetupResponse {
        price_feed: _,
        converter,
        virtual_staking,
    } = setup(
        &app,
        SetupArgs {
            owner,
            admin,
            discount,
            native_per_foreign,
        },
    );

    // no one is staked
    let val1 = "Val Kilmer";
    let val2 = "Valley Girl";
    assert!(virtual_staking.all_stake().unwrap().stakes.is_empty());
    assert_eq!(
        virtual_staking
            .stake(val1.to_string())
            .unwrap()
            .stake
            .u128(),
        0
    );
    assert_eq!(
        virtual_staking
            .stake(val2.to_string())
            .unwrap()
            .stake
            .u128(),
        0
    );

    // let's stake some
    converter
        .test_stake(owner.to_string(), val1.to_string(), coin(1000, JUNO))
        .call(owner)
        .unwrap();
    converter
        .test_stake(owner.to_string(), val2.to_string(), coin(4000, JUNO))
        .call(owner)
        .unwrap();

    // and burn some
    converter
        .test_burn(vec![val2.to_string()], coin(2000, JUNO))
        .call(owner)
        .unwrap();

    // and check the stakes (1000 * 0.6 * 0.5 = 300) (2000 * 0.6 * 0.5 = 600)
    assert_eq!(
        virtual_staking
            .stake(val1.to_string())
            .unwrap()
            .stake
            .u128(),
        300
    );
    assert_eq!(
        virtual_staking
            .stake(val2.to_string())
            .unwrap()
            .stake
            .u128(),
        600
    );
    assert_eq!(
        virtual_staking.all_stake().unwrap().stakes,
        vec![
            (val1.to_string(), Uint128::new(300)),
            (val2.to_string(), Uint128::new(600)),
        ]
    );
}

#[test]
fn valset_update_works() {
    let app = new_app();

    let owner = "sunny"; // Owner of the staking contract (i. e. the vault contract)
    let admin = "theman";
    let discount = Decimal::percent(10); // 1 OSMO worth of JUNO should give 0.9 OSMO of stake
    let native_per_foreign = Decimal::percent(40); // 1 JUNO is worth 0.4 OSMO

    let SetupResponse {
        price_feed: _,
        converter,
        virtual_staking,
    } = setup(
        &app,
        SetupArgs {
            owner,
            admin,
            discount,
            native_per_foreign,
        },
    );

    // Send a valset update
    let add_validators = vec![
        Validator {
            address: "validator1".to_string(),
            commission: Default::default(),
            max_commission: Default::default(),
            max_change_rate: Default::default(),
        },
        Validator {
            address: "validator3".to_string(),
            commission: Default::default(),
            max_commission: Default::default(),
            max_change_rate: Default::default(),
        },
    ];
    let rem_validators = vec!["validator3".to_string()];

    // Check that only the virtual staking contract can call this handler
    let res = converter
        .valset_update(vec![], vec![], vec![], vec![], vec![], vec![], vec![])
        .call(owner);
    assert_eq!(res.unwrap_err(), Unauthorized {});

    let res = converter
        .valset_update(
            add_validators,
            rem_validators,
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
        )
        .call(virtual_staking.contract_addr.as_ref());

    // This fails because of lack of IBC support in mt now.
    // Cannot be tested further in this setup.
    // TODO: Change this when IBC support is there in mt.
    assert_eq!(
        res.unwrap_err(),
        ContractError::Std(StdError::NotFound {
            kind:
                "type: cosmwasm_std::ibc::IbcChannel; key: [69, 62, 63, 5F, 63, 68, 61, 6E, 6E, 65, 6C]"
                    .to_string()
        })
    );
}

#[test]
fn unauthorized() {
    let app = new_app();

    let SetupResponse { converter, .. } = setup(
        &app,
        SetupArgs {
            owner: "owner",
            admin: "admin",
            discount: Decimal::percent(10),
            native_per_foreign: Decimal::percent(40),
        },
    );

    let err = converter
        .distribute_rewards(vec![
            RewardInfo {
                validator: "alice".to_string(),
                reward: 33u128.into(),
            },
            RewardInfo {
                validator: "bob".to_string(),
                reward: 53u128.into(),
            },
        ])
        .call("mallory")
        .unwrap_err();

    assert_eq!(err, ContractError::Unauthorized);

    let err = converter
        .distribute_reward("validator".to_string())
        .call("mallory")
        .unwrap_err();

    assert_eq!(err, ContractError::Unauthorized);

    let err = converter
        .valset_update(vec![], vec![], vec![], vec![], vec![], vec![], vec![])
        .call("mallory")
        .unwrap_err();

    assert_eq!(err, ContractError::Unauthorized);
}

#[test]
fn distribute_rewards_invalid_amount_is_rejected() {
    let owner = "sunny";
    let admin = "theman";
    let discount = Decimal::percent(10); // 1 OSMO worth of JUNO should give 0.9 OSMO of stake
    let native_per_foreign = Decimal::percent(40); // 1 JUNO is worth 0.4 OSMO

    let app = new_app();

    let SetupResponse {
        price_feed: _,
        converter,
        virtual_staking,
    } = setup(
        &app,
        SetupArgs {
            owner,
            admin,
            discount,
            native_per_foreign,
        },
    );

    app.app_mut().init_modules(|router, _, storage| {
        router
            .bank
            .init_balance(
                storage,
                &virtual_staking.contract_addr,
                coins(99999, "TOKEN"),
            )
            .unwrap();
    });

    let err = converter
        .distribute_rewards(vec![
            RewardInfo {
                validator: "alice".to_string(),
                reward: 33u128.into(),
            },
            RewardInfo {
                validator: "bob".to_string(),
                reward: 53u128.into(),
            },
        ])
        .with_funds(&[coin(80, "TOKEN")])
        .call(virtual_staking.contract_addr.as_str())
        .unwrap_err();

    assert_eq!(
        err,
        ContractError::DistributeRewardsInvalidAmount {
            sum: 86u128.into(),
            sent: 80u128.into()
        }
    );

    let err = converter
        .distribute_rewards(vec![
            RewardInfo {
                validator: "alice".to_string(),
                reward: 33u128.into(),
            },
            RewardInfo {
                validator: "bob".to_string(),
                reward: 53u128.into(),
            },
        ])
        .with_funds(&[coin(90, "TOKEN")])
        .call(virtual_staking.contract_addr.as_str())
        .unwrap_err();

    assert_eq!(
        err,
        ContractError::DistributeRewardsInvalidAmount {
            sum: 86u128.into(),
            sent: 90u128.into()
        }
    );
}

#[test]
#[ignore = "IBC unsupported by Sylvia"]
fn distribute_rewards_valid_amount() {
    let owner = "sunny";
    let admin = "theman";
    let discount = Decimal::percent(10); // 1 OSMO worth of JUNO should give 0.9 OSMO of stake
    let native_per_foreign = Decimal::percent(40); // 1 JUNO is worth 0.4 OSMO

    let app = new_app();

    let SetupResponse {
        price_feed: _,
        converter,
        virtual_staking,
    } = setup(
        &app,
        SetupArgs {
            owner,
            admin,
            discount,
            native_per_foreign,
        },
    );

    app.app_mut().init_modules(|router, _, storage| {
        router
            .bank
            .init_balance(
                storage,
                &virtual_staking.contract_addr,
                coins(99999, "TOKEN"),
            )
            .unwrap();
    });

    converter
        .distribute_rewards(vec![
            RewardInfo {
                validator: "alice".to_string(),
                reward: 33u128.into(),
            },
            RewardInfo {
                validator: "bob".to_string(),
                reward: 53u128.into(),
            },
        ])
        .with_funds(&[coin(86, "TOKEN")])
        .call(virtual_staking.contract_addr.as_str())
        .unwrap();
}
