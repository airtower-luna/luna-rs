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


#[cfg(test)]
mod tests {
	use mpsc::RecvError;

	use super::*;

	#[test]
	fn default() -> Result<(), RecvError> {
		let receiver = Generator::Default.run();
		let step = TimeSpec::new(0, 500000000);
		for i in 0..10 {
			let pkt = receiver.recv()?;
			println!("{i} {pkt:?}");
			assert_eq!(pkt.delay, step);
			assert_eq!(pkt.size, MIN_SIZE);
		}
		assert_eq!(receiver.recv(), Err(RecvError));
		Ok(())
	}

	#[test]
	fn vary() -> Result<(), RecvError> {
		let receiver = Generator::Vary.run();
		let step = TimeSpec::new(0, 1000000);
		let size = vec![
			21, 42, 84, 168, 336, 672, 1344, 1500, 1344, 672,
			336, 168, 84, 42, 21, 42, 84, 168, 336, 672];
		for i in 0..20 {
			let pkt = receiver.recv()?;
			println!("{i} {pkt:?}");
			assert_eq!(pkt.delay, step);
			assert_eq!(pkt.size, size[i]);
		}
		assert_eq!(receiver.recv(), Err(RecvError));
		Ok(())
	}
}
