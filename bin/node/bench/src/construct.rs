// This file is part of Substrate.

// Copyright (C) 2020 Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with this program. If not, see <https://www.gnu.org/licenses/>.

//! Block import benchmark.
//!
//! This benchmark is expected to measure block import operation of
//! some more or less full block.
//!
//! As we also want to protect against cold-cache attacks, this
//! benchmark should not rely on any caching (except those that
//! DO NOT depend on user input). Thus block generation should be
//! based on randomized operation.
//!
//! This is supposed to be very simple benchmark and is not subject
//! to much configuring - just block full of randomized transactions.
//! It is not supposed to measure runtime modules weight correctness

use std::{
	borrow::Cow,
	collections::HashMap,
	pin::Pin,
	sync::Arc,
};
use futures::Future;

use node_primitives::Block;
use node_runtime::UncheckedExtrinsic;
use node_testing::bench::{BenchDb, Profile, BlockType, KeyTypes, DatabaseType};
use sp_runtime::{
	generic::BlockId,
	traits::{Block as BlockT, NumberFor},
	OpaqueExtrinsic,
};
use sp_inherents::InherentData;
use sp_transaction_pool::{
	ImportNotificationStream,
	PoolFuture,
	PoolStatus,
	TransactionFor,
	TransactionSource,
	TransactionStatusStreamFor,
	TxHash,
};

use crate::{
	common::SizeType,
	core::{self, Path, Mode},
};

pub struct ConstructionBenchmarkDescription {
	pub profile: Profile,
	pub key_types: KeyTypes,
	pub block_type: BlockType,
	pub size: SizeType,
	pub database_type: DatabaseType,
}

pub struct ConstructionBenchmark {
	profile: Profile,
	database: BenchDb,
	transactions: Transactions,
}

impl core::BenchmarkDescription for ConstructionBenchmarkDescription {
	fn path(&self) -> Path {

		let mut path = Path::new(&["node", "propose"]);

		match self.profile {
			Profile::Wasm => path.push("wasm"),
			Profile::Native => path.push("native"),
		}

		match self.key_types {
			KeyTypes::Sr25519 => path.push("sr25519"),
			KeyTypes::Ed25519 => path.push("ed25519"),
		}

		match self.block_type {
			BlockType::RandomTransfersKeepAlive => path.push("transfer"),
			BlockType::RandomTransfersReaping => path.push("transfer_reaping"),
			BlockType::Noop => path.push("noop"),
		}

		match self.database_type {
			DatabaseType::RocksDb => path.push("rocksdb"),
			DatabaseType::ParityDb => path.push("paritydb"),
		}

		path.push(&format!("{}", self.size));

		path
	}

	fn setup(self: Box<Self>) -> Box<dyn core::Benchmark> {
		let mut extrinsics: Vec<Arc<PoolTransaction>> = Vec::new();

		let mut bench_db = BenchDb::with_key_types(
			self.database_type,
			50_000,
			self.key_types
		);


		let client = bench_db.client();

		for inherent in bench_db.generate_inherents(&client) {
			extrinsics.push(Arc::new(inherent.into()));
		}

		let content_type = self.block_type.to_content(self.size.transactions());
		for transaction in bench_db.block_content(content_type, &client) {
			extrinsics.push(Arc::new(transaction.into()));
		}

		Box::new(ConstructionBenchmark {
			profile: self.profile,
			database: bench_db,
			transactions: Transactions(extrinsics),
		})
	}

	fn name(&self) -> Cow<'static, str> {
		format!(
			"Block construction ({:?}/{}, {:?}, {:?} backend)",
			self.block_type,
			self.size,
			self.profile,
			self.database_type,
		).into()
	}
}

impl core::Benchmark for ConstructionBenchmark {
	fn run(&mut self, mode: Mode) -> std::time::Duration {
		let mut context = self.database.create_context(self.profile);

		let _ = context.client.runtime_version_at(&BlockId::Number(0))
			.expect("Failed to get runtime version")
			.spec_version;

		if mode == Mode::Profile {
			std::thread::park_timeout(std::time::Duration::from_secs(3));
		}

		let mut proposer_factory = sc_basic_authorship::ProposerFactory::new(
			context.client.clone(),
			self.transactions.clone().into(),
			None,
		);

		let start = std::time::Instant::now();

		//
		// Benchmark goes here
		//

		let elapsed = start.elapsed();

		if mode == Mode::Profile {
			std::thread::park_timeout(std::time::Duration::from_secs(1));
		}

		elapsed
	}
}

#[derive(Clone, Debug)]
pub struct PoolTransaction {
	data: OpaqueExtrinsic,
	hash: node_primitives::Hash,
}

impl From<OpaqueExtrinsic> for PoolTransaction {
	fn from(e: OpaqueExtrinsic) -> Self {
		PoolTransaction {
			data: e,
			hash: node_primitives::Hash::zero(),
		}
	}
}

impl sp_transaction_pool::InPoolTransaction for PoolTransaction {
	type Transaction = OpaqueExtrinsic;
	type Hash = node_primitives::Hash;

	fn data(&self) -> &Self::Transaction {
		&self.data
	}

	fn hash(&self) -> &Self::Hash {
		&self.hash
	}

	fn priority(&self) -> &u64 { unimplemented!() }

	fn longevity(&self) -> &u64 { unimplemented!() }

	fn requires(&self) -> &[Vec<u8>] { unimplemented!() }

	fn provides(&self) -> &[Vec<u8>] { unimplemented!() }

	fn is_propagable(&self) -> bool { unimplemented!() }
}

#[derive(Clone, Debug)]
pub struct Transactions(Vec<Arc<PoolTransaction>>);

impl sp_transaction_pool::TransactionPool for Transactions {
	type Block = Block;
	type Hash = node_primitives::Hash;
	type InPoolTransaction = PoolTransaction;
	type Error = sp_transaction_pool::error::Error;

	/// Returns a future that imports a bunch of unverified transactions to the pool.
	fn submit_at(
		&self,
		at: &BlockId<Self::Block>,
		source: TransactionSource,
		xts: Vec<TransactionFor<Self>>,
	) -> PoolFuture<Vec<Result<node_primitives::Hash, Self::Error>>, Self::Error>  {
		unimplemented!()
	}

	/// Returns a future that imports one unverified transaction to the pool.
	fn submit_one(
		&self,
		at: &BlockId<Self::Block>,
		source: TransactionSource,
		xt: TransactionFor<Self>,
	) -> PoolFuture<TxHash<Self>, Self::Error> {
		unimplemented!()
	}

	fn submit_and_watch(
		&self,
		at: &BlockId<Self::Block>,
		source: TransactionSource,
		xt: TransactionFor<Self>,
	) -> PoolFuture<Box<TransactionStatusStreamFor<Self>>, Self::Error> {
		unimplemented!()
	}

	fn ready_at(&self, at: NumberFor<Self::Block>)
		-> Pin<Box<dyn Future<Output=Box<dyn Iterator<Item=Arc<Self::InPoolTransaction>> + Send>> + Send>>
	{
		let iter: Box<dyn Iterator<Item=Arc<PoolTransaction>> + Send> = Box::new(self.0.clone().into_iter());
		Box::pin(futures::future::ready(iter))
	}

	fn ready(&self) -> Box<dyn Iterator<Item=Arc<Self::InPoolTransaction>> + Send> {
		unimplemented!()
	}

	fn remove_invalid(&self, hashes: &[TxHash<Self>]) -> Vec<Arc<Self::InPoolTransaction>> {
		Default::default()
	}

	fn status(&self) -> PoolStatus {
		unimplemented!()
	}

	fn import_notification_stream(&self) -> ImportNotificationStream<TxHash<Self>> {
		unimplemented!()
	}

	fn on_broadcasted(&self, propagations: HashMap<TxHash<Self>, Vec<String>>) {
		unimplemented!()
	}

	fn hash_of(&self, xt: &TransactionFor<Self>) -> TxHash<Self> {
		unimplemented!()
	}

	fn ready_transaction(&self, hash: &TxHash<Self>) -> Option<Arc<Self::InPoolTransaction>> {
		unimplemented!()
	}
}