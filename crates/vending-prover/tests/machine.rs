use vending_prover::backend::{decode_machine, VENDING_PROGRAM_ID};

#[test]
fn machine_reads_exact_revision_two_layout_and_authenticates_owner() {
    let key =
        hex::decode(include_str!("../../vending-guest/fixtures/shop.ivk.hex").trim()).unwrap();
    let machine = bs58::encode([2; 32]).into_string();
    let data = include_bytes!("fixtures/machine-revision2.bin").to_vec();
    assert_eq!(data.len(), 291);
    assert_eq!(&data[..4], &[1, 1, 254, 253]);
    assert_eq!(&data[132..140], &100000u64.to_le_bytes());
    assert_eq!(&data[223..227], &144u32.to_le_bytes());
    let shop = decode_machine(&machine, VENDING_PROGRAM_ID, false, &data).unwrap();
    assert_eq!(shop.shop_viewing_key, hex::encode(&key));
    assert_eq!(shop.relay, "C9eFRXy11aT8JqcyZPvEGqsor3pmrAyPDBmg6LCcGvWS");
    assert_eq!(shop.diversifier, "6f78f490a60243c11a9fad");
    assert!(decode_machine(&machine, &machine, false, &data).is_err());
    assert!(decode_machine(&machine, VENDING_PROGRAM_ID, true, &data).is_err());
    for len in [227, 290, 292] {
        let mut bad = data.clone();
        bad.resize(len, 0);
        assert!(decode_machine(&machine, VENDING_PROGRAM_ID, false, &bad).is_err());
    }
    for at in [0, 1, 159, 227] {
        let mut bad = data.clone();
        bad[at] ^= 1;
        assert!(decode_machine(&machine, VENDING_PROGRAM_ID, false, &bad).is_err());
    }
}

#[tokio::test]
async fn rpc_source_reads_public_key_from_the_machine_account() {
    use base64::{engine::general_purpose::STANDARD, Engine};
    use vending_prover::backend::{RpcShops, ShopSource};
    let key =
        hex::decode(include_str!("../../vending-guest/fixtures/shop.ivk.hex").trim()).unwrap();
    let machine = bs58::encode([2; 32]).into_string();
    let data = include_bytes!("fixtures/machine-revision2.bin");
    let response = serde_json::json!({"jsonrpc":"2.0","id":1,"result":{"value":{"owner":VENDING_PROGRAM_ID,"executable":false,"data":[STANDARD.encode(data),"base64"]}}});
    let expected = machine.clone();
    let app = axum::Router::new().route(
        "/",
        axum::routing::post(move |axum::Json(request): axum::Json<serde_json::Value>| {
            let response = response.clone();
            let expected = expected.clone();
            async move {
                assert_eq!(request["method"], "getAccountInfo");
                assert_eq!(request["params"][0], expected);
                assert_eq!(request["params"][1]["commitment"], "finalized");
                axum::Json(response)
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let source = RpcShops::new(format!("http://{addr}")).unwrap();
    let shop = source.shop(&machine).await.unwrap();
    assert_eq!(shop.shop_viewing_key, hex::encode(key));
    server.abort();
}
