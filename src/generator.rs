use std::sync::mpsc;
use std::thread;

use clap::ValueEnum;
use nix::sys::time::TimeSpec;

use crate::{PacketData, MIN_SIZE};


#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
pub enum Generator {
	/// send a minimum size packet every 0.5s
	Default,
	/// send a minimum size packet every 30µs
	Rapid,
	/// send a 1500 byte packet every 1ms
	Large,
	/// change size between minimum and 1500, send every 1ms
	Vary,
}

impl Generator {
	pub fn run(&self) -> mpsc::Receiver<PacketData> {
		let (sender, receiver) = mpsc::channel::<PacketData>();
		match self {
			Generator::Default => thread::spawn(move || generator(sender)),
			Generator::Large => thread::spawn(move || generator_large(sender)),
			Generator::Rapid => thread::spawn(move || generator_rapid(sender)),
			Generator::Vary => thread::spawn(move || generator_vary_size(sender)),
		};
		receiver
	}
}

fn generator(target: mpsc::Sender<PacketData>) {
	let step = TimeSpec::new(0, 500000000);
	for _ in 0..10 {
		target.send(PacketData { delay: step, size: MIN_SIZE }).unwrap();
	}
}


fn generator_rapid(target: mpsc::Sender<PacketData>) {
	let step = TimeSpec::new(0, 30000);
	for _ in 0..10 {
		target.send(PacketData { delay: step, size: MIN_SIZE }).unwrap();
	}
}


fn generator_large(target: mpsc::Sender<PacketData>) {
	let step = TimeSpec::new(0, 1000000);
	for _ in 0..10 {
		target.send(PacketData { delay: step, size: 1500 }).unwrap();
	}
}


fn generator_vary_size(target: mpsc::Sender<PacketData>) {
	let step = TimeSpec::new(0, 1000000);
	let max_size = 1500;
	let mut s = MIN_SIZE;
	let mut grow = true;
	for _ in 0..20 {
		target.send(PacketData { delay: step, size: max_size.min(s) }).unwrap();
		if grow {
			s *= 2;
			grow = s < max_size;
		} else {
			s = MIN_SIZE.max(s / 2);
			grow = s <= MIN_SIZE;
		}
	}
}
