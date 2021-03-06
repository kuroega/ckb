use crate::error::RPCError;
use ckb_core::transaction::{ProposalShortId, Transaction as CoreTransaction};
use ckb_network::NetworkController;
use ckb_protocol::RelayMessage;
use ckb_shared::shared::Shared;
use ckb_shared::store::ChainStore;
use ckb_shared::tx_pool::types::PoolEntry;
use ckb_sync::NetworkProtocol;
use ckb_traits::chain_provider::ChainProvider;
use flatbuffers::FlatBufferBuilder;
use jsonrpc_core::{Error, Result};
use jsonrpc_derive::rpc;
use jsonrpc_types::Transaction;
use log::debug;
use numext_fixed_hash::H256;
use std::convert::TryInto;

#[rpc]
pub trait PoolRpc {
    // curl -d '{"id": 2, "jsonrpc": "2.0", "method":"send_transaction","params": [{"version":2, "deps":[], "inputs":[], "outputs":[]}]}' -H 'content-type:application/json' 'http://localhost:8114'
    #[rpc(name = "send_transaction")]
    fn send_transaction(&self, _tx: Transaction) -> Result<H256>;

    // curl -d '{"id": 2, "jsonrpc": "2.0", "method":"get_pool_transaction","params": [""]}' -H 'content-type:application/json' 'http://localhost:8114'
    #[rpc(name = "get_pool_transaction")]
    fn get_pool_transaction(&self, _hash: H256) -> Result<Option<Transaction>>;
}

pub(crate) struct PoolRpcImpl<CS> {
    pub network_controller: NetworkController,
    pub shared: Shared<CS>,
}

impl<CS: ChainStore + 'static> PoolRpc for PoolRpcImpl<CS> {
    fn send_transaction(&self, tx: Transaction) -> Result<H256> {
        let tx: CoreTransaction = tx.try_into().map_err(|_| Error::parse_error())?;

        let mut chain_state = self.shared.chain_state().lock();
        let rtx = chain_state.rpc_resolve_tx_from_pool(&tx, &chain_state.tx_pool());
        let tx_result = chain_state.verify_rtx(&rtx, self.shared.consensus().max_block_cycles());
        debug!(target: "rpc", "send_transaction add to pool result: {:?}", tx_result);
        match tx_result {
            Err(err) => Err(RPCError::custom(RPCError::Invalid, format!("{:?}", err))),
            Ok(cycles) => {
                let entry = PoolEntry::new(tx.clone(), 0, Some(cycles));
                let tx_hash = tx.hash().clone();
                if !chain_state.mut_tx_pool().enqueue_tx(entry) {
                    // Duplicate tx
                    Ok(tx_hash)
                } else {
                    let fbb = &mut FlatBufferBuilder::new();
                    let message = RelayMessage::build_transaction(fbb, &tx, cycles);
                    fbb.finish(message, None);
                    let data = fbb.finished_data().to_vec();
                    self.network_controller
                        .broadcast(NetworkProtocol::RELAY.into(), data);
                    Ok(tx_hash)
                }
            }
        }
    }

    fn get_pool_transaction(&self, hash: H256) -> Result<Option<Transaction>> {
        let id = ProposalShortId::from_tx_hash(&hash);
        Ok(self
            .shared
            .chain_state()
            .lock()
            .tx_pool()
            .get_tx(&id)
            .map(|tx| (&tx).into()))
    }
}
