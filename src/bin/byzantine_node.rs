use common_crypto::{
    Crypto, PrivateKey, PublicKey, Secp256k1, Secp256k1PrivateKey, Signature, ToPublicKey,
};
use core_network::{NetworkConfig, NetworkService, NetworkServiceHandle};
use derive_more::Display;
use protocol::{
    async_trait,
    fixed_codec::FixedCodec,
    traits::{Context, Gossip, MessageCodec, MessageHandler, Priority, Rpc, TrustFeedback},
    types::{
        Address, Block, BlockHeader, Hash, Hex, JsonString, Proof, RawTransaction,
        SignedTransaction, TransactionRequest,
    },
    Bytes, BytesMut,
};
use rand::{rngs::OsRng, RngCore};
use serde::Deserialize;

use core_consensus::message::{
    FixedBlock, FixedHeight, RPC_RESP_SYNC_PULL_BLOCK, RPC_SYNC_PULL_BLOCK,
};
use core_mempool::{MsgNewTxs, END_GOSSIP_NEW_TXS};

use std::{convert::TryFrom, net::SocketAddr, ops::Deref};

#[derive(Debug, Display)]
pub enum ClientNodeError {
    #[display(fmt = "not connected")]
    NotConnected,

    #[display(fmt = "unexpected {}", _0)]
    Unexpected(String),
}
impl std::error::Error for ClientNodeError {}

type ClientResult<T> = Result<T, ClientNodeError>;

#[derive(Debug, Clone, Deserialize)]
pub struct ConfigNetwork {
    pub bootstraps:                 Option<Vec<ConfigNetworkBootstrap>>,
    pub whitelist:                  Option<Vec<String>>,
    pub whitelist_peers_only:       Option<bool>,
    pub trust_interval_duration:    Option<u64>,
    pub trust_max_history_duration: Option<u64>,
    pub fatal_ban_duration:         Option<u64>,
    pub soft_ban_duration:          Option<u64>,
    pub max_connected_peers:        Option<usize>,
    pub listening_address:          SocketAddr,
    pub rpc_timeout:                Option<u64>,
    pub selfcheck_interval:         Option<u64>,
    pub send_buffer_size:           Option<usize>,
    pub write_timeout:              Option<u64>,
    pub recv_buffer_size:           Option<usize>,
    pub max_frame_length:           Option<usize>,
    pub max_wait_streams:           Option<usize>,
    pub ping_interval:              Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ConfigNetworkBootstrap {
    pub pubkey:  Hex,
    pub address: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub privkey: Hex,
    pub network: ConfigNetwork,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Genesis {
    pub timestamp: u64,
    pub prevhash:  Hex,
    pub services:  Vec<ServiceParam>,
}

impl Genesis {
    pub fn get_payload(&self, name: &str) -> &str {
        &self
            .services
            .iter()
            .find(|&service| service.name == name)
            .unwrap_or_else(|| panic!("miss {:?} service!", name))
            .payload
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct ServiceParam {
    pub name:    String,
    pub payload: String,
}

#[derive(Deserialize, Clone, Debug)]
pub struct Metadata {
    pub chain_id:        Hash,
    pub common_ref:      Hex,
    pub timeout_gap:     u64,
    pub cycles_limit:    u64,
    pub cycles_price:    u64,
    pub interval:        u64,
    pub propose_ratio:   u64,
    pub prevote_ratio:   u64,
    pub precommit_ratio: u64,
    pub brake_ratio:     u64,
    pub tx_num_limit:    u64,
    pub max_tx_size:     u64,
}

#[derive(Debug, Clone)]
struct FullNodeInfo {
    pubkey:     String,
    chain_addr: Address,
    multiaddr:  String,
}

impl FullNodeInfo {
    pub fn from_conf(config: Config) -> Self {
        let mut bootstraps = config.network.bootstraps.expect("config.toml full node");
        let full_node = bootstraps.pop().expect("there should be one bootstrap");

        let hex_pubkey = full_node.pubkey.as_string_trim0x();
        let pubkey = hex::decode(hex_pubkey.clone()).expect("decode hex full node pubkey");
        let chain_addr =
            Address::from_pubkey_bytes(pubkey.into()).expect("full node chain address");

        FullNodeInfo {
            pubkey: hex_pubkey,
            chain_addr,
            multiaddr: full_node.address,
        }
    }
}

struct DummyPullBlockRpcHandler(NetworkServiceHandle);

#[async_trait]
impl MessageHandler for DummyPullBlockRpcHandler {
    type Message = FixedHeight;

    async fn process(&self, ctx: Context, msg: FixedHeight) -> TrustFeedback {
        let block = FixedBlock::new(mock_block(msg.inner));
        self.0
            .response(ctx, RPC_RESP_SYNC_PULL_BLOCK, Ok(block), Priority::High)
            .await
            .expect("dummy response pull block");

        TrustFeedback::Neutral
    }
}

struct ClientNode {
    pub network:        NetworkServiceHandle,
    pub full_node_info: FullNodeInfo,
    pub priv_key:       Secp256k1PrivateKey,
    pub chain_id:       Hash,

    pub config: Config,
}

impl ClientNode {
    pub async fn make(config_file: String, genesis_file: String) -> Self {
        let config: Config =
            common_config_parser::parse(&config_file).expect("parse chain config.toml");
        let genesis: Genesis =
            common_config_parser::parse(&genesis_file).expect("parse genesis.toml");

        log::info!("config {:?}", config);
        log::info!("genesis {:?}", genesis);

        let full_node_info = FullNodeInfo::from_conf(config.clone());
        let metadata: Metadata =
            serde_json::from_str(genesis.get_payload("metadata")).expect("decode metadata");

        let chain_id = metadata.chain_id;

        let network_config = NetworkConfig::new()
            .bootstraps(vec![(
                full_node_info.pubkey.clone(),
                full_node_info.multiaddr.clone(),
            )])
            .expect("bootstrap node config");
        let self_priv_key = {
            let hex_privkey = hex::decode(config.privkey.as_string_trim0x())
                .expect("hex decode self private key");
            Secp256k1PrivateKey::try_from(hex_privkey.as_ref()).expect("self private key try from")
        };

        let mut network = NetworkService::new(network_config);
        let handle = network.handle();

        network
            .register_endpoint_handler(
                RPC_SYNC_PULL_BLOCK,
                Box::new(DummyPullBlockRpcHandler(handle.clone())),
            )
            .expect("register consensus rpc pull block");
        network
            .register_rpc_response::<FixedBlock>(RPC_RESP_SYNC_PULL_BLOCK)
            .expect("register consensus rpc response pull block");

        network
            .listen(config.network.listening_address)
            .await
            .expect("client node listen");

        tokio::spawn(network);

        let mut count = 30u8;
        while count > 0 {
            count -= 1;
            if handle
                .diagnostic
                .session_by_chain(&full_node_info.chain_addr)
                .is_some()
            {
                break;
            }
            tokio::time::delay_for(std::time::Duration::from_secs(1)).await;
        }
        if count == 0 {
            panic!("failed to connect full node");
        }

        ClientNode {
            network: handle,
            full_node_info,
            priv_key: self_priv_key,
            chain_id,

            config,
        }
    }

    pub fn connected(&self) -> bool {
        self.network
            .diagnostic
            .session_by_chain(&self.full_node_info.chain_addr)
            .is_some()
    }

    pub async fn broadcast<M: MessageCodec>(&self, endpoint: &str, msg: M) -> ClientResult<()> {
        let diagnostic = &self.network.diagnostic;
        let sid = match diagnostic.session_by_chain(&self.full_node_info.chain_addr) {
            Some(sid) => sid,
            None => return Err(ClientNodeError::NotConnected),
        };

        let ctx = Context::new().with_value::<usize>("session_id", sid.value());
        let users = vec![self.full_node_info.chain_addr.clone()];
        if let Err(e) = self
            .users_cast::<M>(ctx, endpoint, users, msg, Priority::High)
            .await
        {
            // Sleep a while to ensure our peer manager to process disconnect event
            tokio::time::delay_for(std::time::Duration::from_secs(2)).await;

            if !self.connected() {
                Err(ClientNodeError::NotConnected)
            } else {
                Err(ClientNodeError::Unexpected(format!(
                    "broadcast to {} {}",
                    endpoint, e
                )))
            }
        } else {
            Ok(())
        }
    }

    pub async fn rpc<M: MessageCodec, R: MessageCodec>(
        &self,
        endpoint: &str,
        msg: M,
    ) -> ClientResult<R> {
        let diagnostic = &self.network.diagnostic;
        let sid = match diagnostic.session_by_chain(&self.full_node_info.chain_addr) {
            Some(sid) => sid,
            None => return Err(ClientNodeError::NotConnected),
        };

        let ctx = Context::new().with_value::<usize>("session_id", sid.value());
        match self.call::<M, R>(ctx, endpoint, msg, Priority::High).await {
            Ok(resp) => Ok(resp),
            Err(e)
                if e.to_string().contains("RpcTimeout")
                    || e.to_string().contains("rpc timeout") =>
            {
                // Sleep a while to ensure our peer manager to process disconnect event
                tokio::time::delay_for(std::time::Duration::from_secs(10)).await;

                if !self.connected() {
                    Err(ClientNodeError::NotConnected)
                } else {
                    Err(ClientNodeError::Unexpected(format!(
                        "rpc to {} {}",
                        endpoint, e
                    )))
                }
            }
            Err(e) => Err(ClientNodeError::Unexpected(format!(
                "rpc to {} {}",
                endpoint, e
            ))),
        }
    }

    pub async fn genesis_block(&self) -> ClientResult<Block> {
        let resp = self
            .rpc::<_, FixedBlock>(RPC_SYNC_PULL_BLOCK, FixedHeight::new(0))
            .await?;
        Ok(resp.inner)
    }
}

impl Deref for ClientNode {
    type Target = NetworkServiceHandle;

    fn deref(&self) -> &Self::Target {
        &self.network
    }
}

#[tokio::main]
pub async fn main() {
    env_logger::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let config_file = std::env::args().nth(1).expect("config file path");
    let genesis_file = std::env::args().nth(2).expect("genesis file path");

    let client_node = ClientNode::make(config_file, genesis_file).await;

    let mut stx = stx_builder(client_node.chain_id.clone()).build(&client_node.priv_key);
    stx.signature = Bytes::from(vec![0]);

    loop {
        'connecting: loop {
            match client_node.genesis_block().await {
                Ok(genesis_block) => {
                    assert_eq!(genesis_block.header.height, 0);
                    log::info!("full node connected");

                    break 'connecting;
                }
                Err(e) => log::error!("genesis {}", e),
            }
        }

        log::info!("send invalid 10 transactions in 1 minute");

        'mempool: for _ in 0..4u8 {
            for _ in 0..10u8 {
                let msg_stxs = MsgNewTxs {
                    batch_stxs: vec![stx.clone()],
                };

                if let Err(e) = client_node.broadcast(END_GOSSIP_NEW_TXS, msg_stxs).await {
                    match e {
                        ClientNodeError::Unexpected(e) => log::error!("unexpect {}", e),
                        ClientNodeError::NotConnected => break 'mempool,
                    }
                }

                // Sleep a while
                tokio::time::delay_for(std::time::Duration::from_secs(2)).await;
            }

            let trust_interval = client_node
                .config
                .network
                .trust_interval_duration
                .unwrap_or_else(|| 60);

            // Sleep until next interval
            tokio::time::delay_for(std::time::Duration::from_secs(trust_interval - 20 + 1)).await;
        }

        let soft_ban_duration = client_node
            .config
            .network
            .soft_ban_duration
            .unwrap_or_else(|| 60 * 10);

        tokio::time::delay_for(std::time::Duration::from_secs(soft_ban_duration * 2)).await;
    }
}

fn mock_block(height: u64) -> Block {
    let block_hash = Hash::digest(Bytes::from("22"));
    let nonce = Hash::digest(Bytes::from("33"));
    let addr_str = "0xCAB8EEA4799C21379C20EF5BAA2CC8AF1BEC475B";

    let proof = Proof {
        height: 0,
        round: 0,
        block_hash,
        signature: Default::default(),
        bitmap: Default::default(),
    };

    let header = BlockHeader {
        chain_id: nonce.clone(),
        height,
        exec_height: height - 1,
        pre_hash: nonce.clone(),
        timestamp: 1000,
        logs_bloom: Default::default(),
        order_root: nonce.clone(),
        confirm_root: Vec::new(),
        state_root: nonce,
        receipt_root: Vec::new(),
        cycles_used: vec![999_999],
        proposer: Address::from_hex(addr_str).unwrap(),
        proof,
        validator_version: 1,
        validators: Vec::new(),
    };

    Block {
        header,
        ordered_tx_hashes: Vec::new(),
    }
}

struct SignedTransactionBuilder {
    chain_id:     Hash,
    timeout:      u64,
    cycles_limit: u64,
    payload:      JsonString,
}

impl SignedTransactionBuilder {
    pub fn new(chain_id: Hash) -> Self {
        let timeout = 19;
        let cycles_limit = 314_159;
        let payload = "test".to_owned();

        SignedTransactionBuilder {
            chain_id,
            timeout,
            cycles_limit,
            payload,
        }
    }

    #[allow(dead_code)]
    pub fn chain_id(mut self, chain_id_bytes: Bytes) -> Self {
        self.chain_id = Hash::digest(chain_id_bytes);
        self
    }

    #[allow(dead_code)]
    pub fn timeout(mut self, timeout: u64) -> Self {
        self.timeout = timeout;
        self
    }

    #[allow(dead_code)]
    pub fn cycles_limit(mut self, cycles_limit: u64) -> Self {
        self.cycles_limit = cycles_limit;
        self
    }

    #[allow(dead_code)]
    pub fn payload(mut self, payload: JsonString) -> Self {
        self.payload = payload;
        self
    }

    pub fn build(self, pk: &Secp256k1PrivateKey) -> SignedTransaction {
        let nonce = {
            let mut random_bytes = [0u8; 32];
            OsRng.fill_bytes(&mut random_bytes);
            Hash::digest(BytesMut::from(random_bytes.as_ref()).freeze())
        };

        let request = TransactionRequest {
            service_name: "metadata".to_owned(),
            method:       "get_metadata".to_owned(),
            payload:      self.payload,
        };

        let raw = RawTransaction {
            chain_id: self.chain_id,
            nonce,
            timeout: self.timeout,
            cycles_limit: self.cycles_limit,
            cycles_price: 1,
            request,
        };

        let raw_bytes = raw.encode_fixed().expect("encode raw tx");
        let tx_hash = Hash::digest(raw_bytes);

        let sig = Secp256k1::sign_message(&tx_hash.as_bytes(), &pk.to_bytes()).expect("sign tx");

        SignedTransaction {
            raw,
            tx_hash,
            pubkey: pk.pub_key().to_bytes(),
            signature: sig.to_bytes(),
        }
    }
}

fn stx_builder(chain_id: Hash) -> SignedTransactionBuilder {
    SignedTransactionBuilder::new(chain_id)
}
