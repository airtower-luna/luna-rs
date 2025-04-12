use std::sync::mpsc;
use std::thread;
#[cfg(feature = "python")]
use std::ffi::{CStr, CString};

use clap::ValueEnum;
use nix::sys::time::TimeSpec;

use crate::{PacketData, MIN_SIZE};


#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
pub enum Generator {
	/// send a minimum size packet every 0.5s
	Default,
	/// send a minimum size packet every 30µs
	Rapid,
	/// send a 1500 byte packet every 1ms
	Large,
	/// change size between minimum and 1500, send every 1ms
	Vary,
	/// load the given string as a Python module and run its
	/// "generate()" function to produce packet data
	#[cfg(feature = "python")]
	#[value(skip)]
	Py(CString),
}

impl Generator {
	pub fn run(self) -> mpsc::Receiver<PacketData> {
		let (sender, receiver) = mpsc::channel::<PacketData>();
		match self {
			Generator::Default => thread::spawn(move || generator(sender)),
			Generator::Large => thread::spawn(move || generator_large(sender)),
			Generator::Rapid => thread::spawn(move || generator_rapid(sender)),
			Generator::Vary => thread::spawn(move || generator_vary_size(sender)),
			#[cfg(feature = "python")]
			Generator::Py(code) => thread::spawn(move || generator_py(&code, sender)),
		};
		receiver
	}
}


fn generator(target: mpsc::Sender<PacketData>) {
	let step = TimeSpec::new(0, 500_000_000);
	for _ in 0..10 {
		target.send(PacketData { delay: step, size: MIN_SIZE }).unwrap();
	}
}


fn generator_rapid(target: mpsc::Sender<PacketData>) {
	let step = TimeSpec::new(0, 30_000);
	for _ in 0..10 {
		target.send(PacketData { delay: step, size: MIN_SIZE }).unwrap();
	}
}


fn generator_large(target: mpsc::Sender<PacketData>) {
	let step = TimeSpec::new(0, 1_000_000);
	for _ in 0..10 {
		target.send(PacketData { delay: step, size: 1500 }).unwrap();
	}
}


fn generator_vary_size(target: mpsc::Sender<PacketData>) {
	let step = TimeSpec::new(0, 1_000_000);
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


#[cfg(feature = "python")]
fn generator_py(generator_code: &CStr, target: mpsc::Sender<PacketData>) {
    use pyo3::exceptions::PyConnectionAbortedError;
    use pyo3::prelude::*;
	use pyo3::ffi::c_str;

	pyo3::prepare_freethreaded_python();
	Python::with_gil(|py| {
		let generator = PyModule::from_code(
			py,
			generator_code,
			c_str!("generator.py"),
			c_str!("generator"),
		)?;
		generator.setattr("MIN_SIZE", MIN_SIZE)?;
		let method = generator.getattr("generate")?;
		let i = method.call0()?;
		i.try_iter()?
			.map(|t|
				 t.and_then(|x| x.extract::<((i64, i64), usize)>()))
			.try_for_each(|t| {
				let ((sec, nsec), size) = t?;
				let step = TimeSpec::new(sec, nsec);
				target.send(PacketData { delay: step, size })
					.map_err(|_| PyConnectionAbortedError::new_err(
						"client thread closed connection"))
			})?;
		PyResult::Ok(())
	}).inspect_err(|e| eprintln!("Generator module failed: {}", e)).unwrap();
}


#[cfg(test)]
mod tests {
	use mpsc::RecvError;

	use super::*;

	#[test]
	fn default() -> Result<(), RecvError> {
		let receiver = Generator::Default.run();
		let step = TimeSpec::new(0, 500_000_000);
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
		let step = TimeSpec::new(0, 1_000_000);
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

	#[cfg(feature = "python")]
	#[test]
	fn py_gen() -> Result<(), Box<dyn std::error::Error>> {
		let code = CString::new(include_str!(concat!(
			env!("CARGO_MANIFEST_DIR"),
			"/examples/generator_random.py"
		)))?;
		let receiver = Generator::Py(code).run();
		let step = TimeSpec::new(0, 1_000_000);
		for i in 0..10 {
			let pkt = receiver.recv()?;
			println!("{i} {pkt:?}");
			assert_eq!(pkt.delay, step);
			assert!(pkt.size >= MIN_SIZE);
			assert!(pkt.size <= 512);
		}
		assert_eq!(receiver.recv(), Err(RecvError));
		Ok(())
	}
}
