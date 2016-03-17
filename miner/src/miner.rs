// Copyright 2015, 2016 Ethcore (UK) Ltd.
// This file is part of Parity.

// Parity is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// Parity is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with Parity.  If not, see <http://www.gnu.org/licenses/>.

use rayon::prelude::*;
use std::sync::{Mutex, RwLock, Arc};
use std::sync::atomic;
use std::sync::atomic::AtomicBool;

use util::{H256, U256, Address, Bytes, Uint};
use ethcore::views::{BlockView};
use ethcore::client::{BlockChainClient, BlockId};
use ethcore::block::{ClosedBlock};
use ethcore::error::{Error};
use ethcore::transaction::SignedTransaction;
use super::{MinerService, MinerStatus, TransactionQueue};

/// Keeps track of transactions using priority queue and holds currently mined block.
pub struct Miner {
	transaction_queue: Mutex<TransactionQueue>,

	// for sealing...
	sealing_enabled: AtomicBool,
	sealing_block: Mutex<Option<ClosedBlock>>,
	gas_floor_target: RwLock<U256>,
	author: RwLock<Address>,
	extra_data: RwLock<Bytes>,

}

impl Default for Miner {
	fn default() -> Miner {
		Miner {
			transaction_queue: Mutex::new(TransactionQueue::new()),
			sealing_enabled: AtomicBool::new(false),
			sealing_block: Mutex::new(None),
			gas_floor_target: RwLock::new(U256::zero()),
			author: RwLock::new(Address::default()),
			extra_data: RwLock::new(Vec::new()),
		}
	}
}

impl Miner {
	/// Creates new instance of miner
	pub fn new() -> Arc<Miner> {
		Arc::new(Miner::default())
	}

	/// Get the author that we will seal blocks as.
	fn author(&self) -> Address {
		*self.author.read().unwrap()
	}

	/// Get the extra_data that we will seal blocks wuth.
	fn extra_data(&self) -> Bytes {
		self.extra_data.read().unwrap().clone()
	}

	/// Get the extra_data that we will seal blocks wuth.
	fn gas_floor_target(&self) -> U256 {
		self.gas_floor_target.read().unwrap().clone()
	}

	/// Set the author that we will seal blocks as.
	pub fn set_author(&self, author: Address) {
		*self.author.write().unwrap() = author;
	}

	/// Set the extra_data that we will seal blocks with.
	pub fn set_extra_data(&self, extra_data: Bytes) {
		*self.extra_data.write().unwrap() = extra_data;
	}

	/// Set the gas limit we wish to target when sealing a new block.
	pub fn set_gas_floor_target(&self, target: U256) {
		*self.gas_floor_target.write().unwrap() = target;
	}

	/// Set minimal gas price of transaction to be accepted for mining.
	pub fn set_minimal_gas_price(&self, min_gas_price: U256) {
		self.transaction_queue.lock().unwrap().set_minimal_gas_price(min_gas_price);
	}
}

impl MinerService for Miner {

	fn clear_and_reset(&self, chain: &BlockChainClient) {
		self.transaction_queue.lock().unwrap().clear();
		self.prepare_sealing(chain);
	}

	fn status(&self) -> MinerStatus {
		let status = self.transaction_queue.lock().unwrap().status();
		MinerStatus {
			transaction_queue_pending: status.pending,
			transaction_queue_future: status.future,
		}
	}

	fn import_transactions<T>(&self, transactions: Vec<SignedTransaction>, fetch_nonce: T) -> Result<(), Error>
		where T: Fn(&Address) -> U256 {
		let mut transaction_queue = self.transaction_queue.lock().unwrap();
		transaction_queue.add_all(transactions, fetch_nonce)
	}

	fn pending_transactions_hashes(&self) -> Vec<H256> {
		let transaction_queue = self.transaction_queue.lock().unwrap();
		transaction_queue.pending_hashes()
	}

	fn prepare_sealing(&self, chain: &BlockChainClient) {
		let no_of_transactions = 128;
		// TODO: should select transactions orm queue according to gas limit of block.
		let transactions = self.transaction_queue.lock().unwrap().top_transactions(no_of_transactions);

		let b = chain.prepare_sealing(
			self.author(),
			self.gas_floor_target(),
			self.extra_data(),
			transactions,
		);
		*self.sealing_block.lock().unwrap() = b;
	}

	fn sealing_block(&self, chain: &BlockChainClient) -> &Mutex<Option<ClosedBlock>> {
		if self.sealing_block.lock().unwrap().is_none() {
			self.sealing_enabled.store(true, atomic::Ordering::Relaxed);
			// TODO: Above should be on a timer that resets after two blocks have arrived without being asked for.
			self.prepare_sealing(chain);
		}
		&self.sealing_block
	}

	fn submit_seal(&self, chain: &BlockChainClient, pow_hash: H256, seal: Vec<Bytes>) -> Result<(), Error> {
		let mut maybe_b = self.sealing_block.lock().unwrap();
		match *maybe_b {
			Some(ref b) if b.hash() == pow_hash => {}
			_ => { return Err(Error::PowHashInvalid); }
		}

		let b = maybe_b.take();
		match chain.try_seal(b.unwrap(), seal) {
			Err(old) => {
				*maybe_b = Some(old);
				Err(Error::PowInvalid)
			}
			Ok(sealed) => {
				// TODO: commit DB from `sealed.drain` and make a VerifiedBlock to skip running the transactions twice.
				try!(chain.import_block(sealed.rlp_bytes()));
				Ok(())
			}
		}
	}

	fn chain_new_blocks(&self, chain: &BlockChainClient, imported: &[H256], invalid: &[H256], enacted: &[H256], retracted: &[H256]) {
		fn fetch_transactions(chain: &BlockChainClient, hash: &H256) -> Vec<SignedTransaction> {
			let block = chain
				.block(BlockId::Hash(*hash))
				// Client should send message after commit to db and inserting to chain.
				.expect("Expected in-chain blocks.");
			let block = BlockView::new(&block);
			block.transactions()
		}

		{
			let in_chain = vec![imported, enacted, invalid];
			let in_chain = in_chain
				.par_iter()
				.flat_map(|h| h.par_iter().map(|h| fetch_transactions(chain, h)));
			let out_of_chain = retracted
				.par_iter()
				.map(|h| fetch_transactions(chain, h));

			in_chain.for_each(|txs| {
				let mut transaction_queue = self.transaction_queue.lock().unwrap();
				let hashes = txs.iter().map(|tx| tx.hash()).collect::<Vec<H256>>();
				transaction_queue.remove_all(&hashes, |a| chain.nonce(a));
			});
			out_of_chain.for_each(|txs| {
				// populate sender
				for tx in &txs {
					let _sender = tx.sender();
				}
				let mut transaction_queue = self.transaction_queue.lock().unwrap();
				let _ = transaction_queue.add_all(txs, |a| chain.nonce(a));
			});
		}

		if self.sealing_enabled.load(atomic::Ordering::Relaxed) {
			self.prepare_sealing(chain);
		}
	}
}