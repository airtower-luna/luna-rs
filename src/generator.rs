use std::{
	collections::HashMap,
	fmt,
	sync::mpsc,
	thread,
};
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
	pub fn run(
		self, options: HashMap<String, String>)
		-> Result<mpsc::Receiver<PacketData>, std::io::Error>
	{
		let (sender, receiver) = mpsc::channel::<PacketData>();
		let t = thread::Builder::new().name(format!("{}", self));
		match self {
			Generator::Default => t.spawn(move || generator(sender, options))?,
			Generator::Large => t.spawn(move || generator_large(sender, options))?,
			Generator::Rapid => t.spawn(move || generator_rapid(sender, options))?,
			Generator::Vary => t.spawn(move || generator_vary_size(sender, options))?,
			#[cfg(feature = "python")]
			Generator::Py(code) => t.spawn(move || generator_py(&code, sender, options))?,
		};
		Ok(receiver)
	}
}


impl fmt::Display for Generator {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Generator::Default => write!(f, "Generator::Default"),
			Generator::Large => write!(f, "Generator::Large"),
			Generator::Rapid => write!(f, "Generator::Rapid"),
			Generator::Vary => write!(f, "Generator::Vary"),
			#[cfg(feature = "python")]
			Generator::Py(_) => write!(f, "Generator::Py(...)"),
		}
	}
}


fn get_count(options: &HashMap<String, String>, default: usize) -> usize {
	options.get("count")
		.map(|s| s.parse().expect("invalid count value"))
		.unwrap_or(default)
}


fn generator(
	target: mpsc::Sender<PacketData>, options: HashMap<String, String>)
{
	let count = get_count(&options, 10);
	let step = TimeSpec::new(0, 500_000_000);
	for _ in 0..count {
		target.send(PacketData { delay: step, size: MIN_SIZE }).unwrap();
	}
}


fn generator_rapid(
	target: mpsc::Sender<PacketData>, options: HashMap<String, String>)
{
	let count = get_count(&options, 200);
	let nsec = options.get("nsec")
		.map(|s| s.parse().expect("invalid nsec value"))
		.unwrap_or(30_000);
	let step = TimeSpec::new(0, nsec);
	for _ in 0..count {
		target.send(PacketData { delay: step, size: MIN_SIZE }).unwrap();
	}
}


fn generator_large(
	target: mpsc::Sender<PacketData>, options: HashMap<String, String>)
{
	let count = get_count(&options, 10);
	let step = TimeSpec::new(0, 1_000_000);
	for _ in 0..count {
		target.send(PacketData { delay: step, size: 1500 }).unwrap();
	}
}


fn generator_vary_size(
	target: mpsc::Sender<PacketData>, options: HashMap<String, String>)
{
	let count = get_count(&options, 20);
	let step = TimeSpec::new(0, 1_000_000);
	let max_size = 1500;
	let mut s = MIN_SIZE;
	let mut grow = true;
	for _ in 0..count {
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
fn generator_py(
	generator_code: &CStr, target: mpsc::Sender<PacketData>,
	options: HashMap<String, String>)
{
    use pyo3::exceptions::PyConnectionAbortedError;
    use pyo3::prelude::*;
	use pyo3::ffi::c_str;
	use pyo3::types::PyDict;

	pyo3::prepare_freethreaded_python();
	Python::with_gil(|py| {
		let generator = PyModule::from_code(
			py,
			generator_code,
			c_str!("generator.py"),
			c_str!("generator"),
		)?;
		generator.setattr("MIN_SIZE", MIN_SIZE)?;
		let pyoptions = PyDict::new(py);
		for (key, value) in options.iter() {
			pyoptions.set_item(key, value)?;
		}
		let method = generator.getattr("generate")?;
		let i = method.call1((pyoptions,))?;
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
	fn default() -> Result<(), Box<dyn std::error::Error>> {
		let mut options = HashMap::with_capacity(1);
		options.insert(String::from("count"), String::from("20"));
		let receiver = Generator::Default.run(options)?;
		let step = TimeSpec::new(0, 500_000_000);
		for i in 0..20 {
			let pkt = receiver.recv()?;
			println!("{i} {pkt:?}");
			assert_eq!(pkt.delay, step);
			assert_eq!(pkt.size, MIN_SIZE);
		}
		assert_eq!(receiver.recv(), Err(RecvError));
		Ok(())
	}

	#[test]
	fn vary() -> Result<(), Box<dyn std::error::Error>> {
		let options = HashMap::new();
		let receiver = Generator::Vary.run(options)?;
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
		let mut options = HashMap::new();
		let count = 256;
		options.insert(String::from("count"), format!("{count}"));
		let receiver = Generator::Py(code).run(options)?;
		let step = TimeSpec::new(0, 1_000_000);
		for i in 0..count {
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
