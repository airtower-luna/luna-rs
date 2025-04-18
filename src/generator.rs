use std::{
	collections::HashMap,
	fmt::{self, Debug, Display},
	num::ParseIntError,
	ops::Deref,
	sync::mpsc,
	thread,
	time::Duration
};
#[cfg(feature = "python")]
use std::ffi::{CStr, CString};

use clap::ValueEnum;
use nix::sys::time::TimeSpec;

use crate::{PacketData, MIN_SIZE};


#[derive(Debug)]
pub struct InvalidOption {
	pub option: String,
	pub source: Box<dyn std::error::Error>,
}

impl Display for InvalidOption {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "Option \"{}\" has an invalid value", self.option)
	}
}

impl std::error::Error for InvalidOption {
	fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
		Some(self.source.deref())
	}
}


#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
pub enum Generator {
	/// send fixed size packet with a fixed interval, defaults to
	/// minimum size and 0.5s
	Default,
	/// change size between minimum and 1500, send every 1ms
	Vary,
	/// load the given string as a Python module and run its
	/// "generate()" function to produce packet data
	#[cfg(feature = "python")]
	#[value(skip)]
	Py{code: CString, file: CString},
}

impl Generator {
	pub fn run(
		self, options: HashMap<String, String>)
		-> Result<mpsc::Receiver<PacketData>, Box<dyn std::error::Error>>
	{
		let (sender, receiver) = mpsc::channel::<PacketData>();
		match self {
			Generator::Default => generator(sender, options)?,
			Generator::Vary => generator_vary_size(sender, options)?,
			#[cfg(feature = "python")]
			Generator::Py{code, file} =>
				thread::Builder::new()
					.name(format!("python generator ({:?})", file))
					.spawn(move || generator_py(&code, &file, sender, options).unwrap())?,
		};
		Ok(receiver)
	}
}


impl fmt::Display for Generator {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Generator::Default => write!(f, "Generator::Default"),
			Generator::Vary => write!(f, "Generator::Vary"),
			#[cfg(feature = "python")]
			Generator::Py{code:_, file} => write!(f, "Generator::Py({:?})", file),
		}
	}
}


macro_rules! parse_or_default {
	($hash:expr, $key:literal, $default:expr) => {{
		$hash.get($key).map(|s| s.parse())
			.transpose()
			.map_err(|e| InvalidOption {
				option: $key.to_string(),
				source: Box::new(e)
			})?
			.unwrap_or($default)
	}};
}


fn parse_timespec(value: &str) -> Result<TimeSpec, ParseIntError> {
	let t = value.split_once('.')
		.or(Some((value, "")))
		.map(
			|v| (
				match v.0 { "" => "0", s => s },
				format!("{:0<9}", v.1)))
		// `or` above ensures the Option is not None here
		.unwrap();
	Ok(TimeSpec::new(t.0.parse()?, t.1[..9].parse()?))
}


fn parse_interval(
	options: &HashMap<String, String>)
	-> Result<Option<TimeSpec>, InvalidOption>
{
	let params = ["interval", "msec", "usec", "nsec"];
	let m: Vec<&str> = params.iter()
		.map(|p| *p)
		.filter(|p| options.contains_key(*p))
		.collect();
	if m.len() == 0 {
		return Ok(None);
	}
	if m.len() > 1 {
		return Err(InvalidOption{
			option: m[1].to_string(),
			source: format!("only one of {:?} may be specified", params).into()
		});
	}
	let t: TimeSpec = match m[0] {
		"msec" => Duration::from_millis(parse_or_default!(options, "msec", 0)).into(),
		"usec" => Duration::from_micros(parse_or_default!(options, "usec", 0)).into(),
		"nsec" => Duration::from_nanos(parse_or_default!(options, "nsec", 0)).into(),
		_ => parse_timespec(options.get("interval").unwrap())
			.map_err(|e| InvalidOption {
				option: "interval".to_string(),
				source: Box::new(e)
			})?,
	};
	Ok(Some(t))
}


fn generator(
	target: mpsc::Sender<PacketData>, options: HashMap<String, String>)
	-> Result<thread::JoinHandle<()>, Box<dyn std::error::Error>>
{
	let count = parse_or_default!(options, "count", 10);
	let size = parse_or_default!(options, "size", MIN_SIZE);
	let delay = parse_interval(&options)?
		.unwrap_or(TimeSpec::new(0, 500_000_000));
	Ok(thread::Builder::new()
		.name("default generator".to_string())
		.spawn(move || {
			for _ in 0..count {
				target.send(PacketData { delay, size }).unwrap();
			}
		})?)
}


fn generator_vary_size(
	target: mpsc::Sender<PacketData>, options: HashMap<String, String>)
	-> Result<thread::JoinHandle<()>, Box<dyn std::error::Error>>
{
	let count = parse_or_default!(options, "count", 20);
	let delay = parse_interval(&options)?
		.unwrap_or(TimeSpec::new(0, 1_000_000));
	let max_size = parse_or_default!(options, "max-size", 1452);
	Ok(thread::Builder::new()
		.name("vary generator".to_string())
		.spawn(move || {
			let mut s = MIN_SIZE;
			let mut grow = true;
			for _ in 0..count {
				target.send(
					PacketData { delay, size: max_size.min(s) }
				).unwrap();
				if grow {
					s *= 2;
					grow = s < max_size;
				} else {
					s = MIN_SIZE.max(s / 2);
					grow = s <= MIN_SIZE;
				}
			}
		})?)
}


#[cfg(feature = "python")]
fn generator_py(
	generator_code: &CStr, generator_file: &CStr,
	target: mpsc::Sender<PacketData>, options: HashMap<String, String>)
	-> Result<(), pyo3::PyErr>
{
    use pyo3::exceptions::PyConnectionAbortedError;
    use pyo3::prelude::*;
	use pyo3::ffi::c_str;

	pyo3::prepare_freethreaded_python();
	Python::with_gil(|py| {
		let generator = PyModule::from_code(
			py,
			generator_code,
			generator_file,
			c_str!("generator"),
		)?;
		generator.setattr("MIN_SIZE", MIN_SIZE)?;
		let method = generator.getattr("generate")?;
		let i = method.call1((options,))?;
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
	}).inspect_err(
		|e| Python::with_gil(|py| {
			eprintln!(
				"Generator module failed: {}{}",
				e.traceback(py)
					.map(|t| t.format()
						 .expect("formatting Python traceback failed"))
					.unwrap_or(String::new()),
				e,
			);
		}))
}


#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn default() -> Result<(), Box<dyn std::error::Error>> {
		let count = 20;
		let size = 32;
		let mut options = HashMap::with_capacity(1);
		options.insert("count".to_string(), format!("{count}"));
		options.insert("size".to_string(), format!("{size}"));
		options.insert("interval".to_string(), "0.2".to_string());
		let receiver = Generator::Default.run(options)?;
		let step = TimeSpec::new(0, 200_000_000);
		for i in 0..count {
			let pkt = receiver.recv()?;
			println!("{i} {pkt:?}");
			assert_eq!(pkt.delay, step);
			assert_eq!(pkt.size, size);
		}
		assert_eq!(receiver.recv(), Err(mpsc::RecvError));
		Ok(())
	}

	#[test]
	fn vary() -> Result<(), Box<dyn std::error::Error>> {
		let mut options = HashMap::new();
		options.insert("max-size".to_string(), "3000".to_string());
		let receiver = Generator::Vary.run(options)?;
		let step = TimeSpec::new(0, 1_000_000);
		let size = vec![
			21, 42, 84, 168, 336, 672, 1344, 2688, 3000, 2688,
			1344, 672, 336, 168, 84, 42, 21, 42, 84, 168];
		for i in 0..20 {
			let pkt = receiver.recv()?;
			println!("{i} {pkt:?}");
			assert_eq!(pkt.delay, step);
			assert_eq!(pkt.size, size[i]);
		}
		assert_eq!(receiver.recv(), Err(mpsc::RecvError));
		Ok(())
	}

	#[cfg(feature = "python")]
	#[test]
	fn py_gen() -> Result<(), Box<dyn std::error::Error>> {
		let file = CString::new(concat!(
			env!("CARGO_MANIFEST_DIR"),
			"/examples/generator_random.py"
		))?;
		let code = CString::new(include_str!(concat!(
			env!("CARGO_MANIFEST_DIR"),
			"/examples/generator_random.py"
		)))?;
		let mut options = HashMap::new();
		let count = 256;
		options.insert(String::from("count"), format!("{count}"));
		let receiver = Generator::Py{code, file}.run(options)?;
		let step = TimeSpec::new(0, 1_000_000);
		for i in 0..count {
			let pkt = receiver.recv()?;
			println!("{i} {pkt:?}");
			assert_eq!(pkt.delay, step);
			assert!(pkt.size >= MIN_SIZE);
			assert!(pkt.size <= 512);
		}
		assert_eq!(receiver.recv(), Err(mpsc::RecvError));
		Ok(())
	}

	#[test]
	fn timespec() {
		assert_eq!(parse_timespec(".002"), Ok(TimeSpec::new(0, 2_000_000)));
		assert_eq!(parse_timespec("1.02"), Ok(TimeSpec::new(1, 20_000_000)));
		assert_eq!(parse_timespec("1"), Ok(TimeSpec::new(1, 0)));
		assert_eq!(parse_timespec("0.000000006"), Ok(TimeSpec::new(0, 6)));
		// empty string is equivalent to all zeroes
		assert_eq!(parse_timespec(""), Ok(TimeSpec::new(0, 0)));
		// decimal places beyond the 9th are cut off
		assert_eq!(parse_timespec("1.0000000006"), Ok(TimeSpec::new(1, 0)));
		assert!(parse_timespec("ab.0").is_err());
	}

	#[test]
	fn time_definition() -> Result<(), Box<dyn std::error::Error>> {
		let mut go = HashMap::new();
		assert_eq!(parse_interval(&go)?, None);
		go.insert("msec".to_string(), "1".to_string());
		go.insert("usec".to_string(), "1000".to_string());
		assert!(parse_interval(&go).is_err());
		go.remove("msec");
		assert_eq!(parse_interval(&go)?, Some(TimeSpec::new(0, 1_000_000)));
		go.clear();
		go.insert("interval".to_string(), "0.001".to_string());
		assert_eq!(parse_interval(&go)?, Some(TimeSpec::new(0, 1_000_000)));
		Ok(())
	}
}
