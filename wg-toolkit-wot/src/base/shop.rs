//! The shop: prices, exchange rates and costs of the game, sent in answer to
//! `CMD_SYNC_SHOP` as a zlib-compressed pickled dict that the client keeps in its
//! persistent cache, and read through `Shop.py` and the GUI's `ShopRequester`.
//!
//! The constants are the EU server's, the items' prices those of the game's definitions.

use std::collections::BTreeSet;
use std::io;

use wgtk::pickle::{Global, Value};

use crate::wot::item_prices::ItemPrices;
use crate::wot::item_defs::NATIONS;

use super::account;


/// The shop, as streamed.
#[derive(Debug)]
pub struct Shop {
    pub data: Vec<u8>,
}

impl Shop {

    pub fn new(prices: &ItemPrices) -> io::Result<Self> {
        Ok(Self { data: account::compress(&shop(prices))? })
    }

    /// If the client's cached shop, given by the `(len, crc32)` of its data, is this one.
    pub fn is_cached(&self, len: i64, crc32: i64) -> bool {
        len == self.data.len() as i64 && crc32 == i64::from(crc32fast::hash(&self.data) as i32)
    }

}

fn shop(prices: &ItemPrices) -> Value {

    let per_nation = |value: Value| Value::List(vec![value; NATIONS.len()]);
    let empty_set = || Value::Set(BTreeSet::new());

    let item_prices = Value::Dict(prices.prices.iter()
        .map(|(cd, price)| (Value::from(cd.to_int()), Value::dict([(price.currency, Value::from(price.amount))])))
        .collect());
    let item_prices = Value::object(Global::new("items", "ItemsPrices"), Value::dict([("_itemsPriceInfo", item_prices)]));
    let not_in_shop = Value::Set(prices.not_in_shop.iter().map(|cd| Value::from(cd.to_int())).collect());

    wgtk::pickle!({
        "rev": 1,
        "battlePassLevelCost": {"gold": 250},
        "berthsPrices": (16, 16, [300]),
        "camouflageCost": {0: (250, 1), 7: (25000, 0), 30: (100000, 0)},
        "changeRoleCost": 0,
        "clanCreationCost": 2500,
        "crystalExchangeRate": 200,
        "customRoleSlotChangeCost": {
            6: {"credits": 10000}, 7: {"credits": 25000}, 8: {"credits": 50000},
            9: {"credits": 100000}, 10: {"credits": 150000}, 11: {"credits": 200000},
        },
        "dailyXPFactor": 2,
        "defaults": {"goodies": {"prices": {}}, "items": {}},
        "dropSkillsCost": {
            0: {"credits": 0, "gold": 0, "recertificationForm": 0, "xpReuseFraction": 0.8},
            1: {"credits": 20000, "gold": 0, "recertificationForm": 0, "xpReuseFraction": 0.9},
            2: {"credits": 0, "gold": 200, "recertificationForm": 0, "xpReuseFraction": 1.0},
            3: {"credits": 0, "gold": 0, "recertificationForm": 1, "xpReuseFraction": 1.0},
            4: {"credits": 100000, "gold": 0, "recertificationForm": 0, "xpReuseFraction": 1.0},
        },
        "exchangeRate": 400,
        "exchangeRateForShellsAndEqs": 400,
        "freeXPConversion": (25, 1),
        "freeXPToTManXPRate": 5,
        "goodies": {"goodies": {}, "notInShop": (empty_set()), "prices": {}},
        "isEnabledBuyingGoldEqsForCredits": true,
        "isEnabledBuyingGoldShellsForCredits": true,
        "items": {
            "camouflagePriceFactors": (per_nation(wgtk::pickle!({}))),
            "inscriptionGroupPriceFactors": (per_nation(wgtk::pickle!({}))),
            "itemPrices": item_prices,
            "notInShopCamouflages": (per_nation(empty_set())),
            "notInShopInscriptionGroups": (per_nation(empty_set())),
            "notInShopItems": not_in_shop,
            "notInShopPlayerEmblemGroups": (empty_set()),
            "notInShopProgressionLvlItems": {},
            "operationPrices": {},
            "playerEmblemGroupPriceFactors": {},
            "progressionLvlPrices": {},
            "vehicleCamouflagePriceFactors": {},
            "vehicleSellPriceFactors": {},
            "vehiclesNotToBuy": (empty_set()),
            "vehiclesRentPrices": {},
            "vehiclesToSellForGold": (empty_set()),
        },
        "paidDeluxeRemovalCost": {"crystal": 200},
        "paidModernized1RemovalCost": {"gold": 10},
        "paidModernized2RemovalCost": {"equipCoin": 50},
        "paidModernized3RemovalCost": {"equipCoin": 100},
        "paidRemovalCost": {"gold": 10},
        "paidTrophyBasicRemovalCost": {"gold": 10},
        "paidTrophyUpgradedRemovalCost": {"gold": 10},
        "playerEmblemCost": {0: (15, 1), 7: (1500, 0), 30: (6000, 0)},
        "playerInscriptionCost": {"nations": {}, 0: (15, 1), 7: (1500, 0), 30: (6000, 0)},
        "premiumCost": {1: 250, 3: 650, 7: 1250, 14: 1800, 30: 2500, 90: 7500, 180: 13500, 360: 24000},
        "restore_config": {
            "tankmen": {
                "creditsCost": 60000, "creditsDuration": 2764800, "freeDuration": 172800,
                "goldCost": 20, "goldDuration": 2764800, "limit": 100,
            },
            "vehicles": {"actionCooldown": 259200, "premiumDuration": 259200, "sellToRestoreFactor": 1.1},
        },
        "sellPriceFactor": 0.5,
        "slotsPrices": (12, [("credits", 250000)]),
        "specialPrices": {},
        "spgRedesignFeatures": {"markTargetAreaEnabled": true, "stunEnabled": true},
        "tankman": {
            "recruit": {"options": ({"credits": 0, "gold": 0, "xp": 0},)},
            "retrain": {"options": (
                {"discounts": {}, "skillsEfficiency": 0.3, "skillsEfficiencyWithRoleChange": (-1.0)},
                {"discounts": {"perk_0": 0.0, "perk_1": 0.5}, "skillsEfficiency": 0.6, "skillsEfficiencyWithRoleChange": 0.4},
                {"discounts": {"perk_0": 0.0, "perk_1": 0.5}, "skillsEfficiency": 1.0, "skillsEfficiencyWithRoleChange": 1.0},
            )},
        },
        "tankmanCost": (
            {"baseRoleLoss": 0.2, "classChangeRoleLoss": 0.2, "credits": 0, "gold": 0, "isPremium": false, "roleLevel": 50},
            {"baseRoleLoss": 0.1, "classChangeRoleLoss": 0.1, "credits": 20000, "gold": 0, "isPremium": false, "roleLevel": 75},
            {"baseRoleLoss": 0.0, "classChangeRoleLoss": 0.0, "credits": 0, "gold": 200, "isPremium": true, "roleLevel": 100},
        ),
        "winXPFactorMode": 0,
    })

}
