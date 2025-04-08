use std::{net::{SocketAddr, ToSocketAddrs}, sync::{mpsc::{self, RecvError}, Mutex}, thread};

use luna_rs::{client, PacketData, ReceivedPacket, MIN_SIZE};
use nix::sys::time::TimeSpec;
use pyo3::{exceptions::{PyException, PyValueError}, prelude::*};

#[pyclass(frozen)]
struct Client {
	server: SocketAddr,
	#[pyo3(get)]
	buffer_size: usize,
	#[pyo3(get)]
	echo: bool,
	generator: Mutex<Option<mpsc::Sender<PacketData>>>,
	running: Mutex<Option<thread::JoinHandle<Result<(), String>>>>,
}

#[pyclass(frozen)]
struct LogIter {
	echo_receiver: Mutex<mpsc::Receiver<ReceivedPacket>>,
}

#[pymethods]
impl Client {
	#[new]
	#[pyo3(signature = (server, buffer_size=1500, echo=true))]
	fn new(server: &str, buffer_size: usize, echo: bool) -> PyResult<Self> {
		let server_addr = match server.to_socket_addrs() {
			Err(_) => return Err(PyValueError::new_err("could not resolve address")),
			Ok(mut s) => match s.next() {
				None => return Err(
					PyValueError::new_err("could not resolve address")),
				Some(s) => s,
			}
		};
		Ok(Client {
			server: server_addr,
			buffer_size,
			echo,
			generator: Mutex::new(None),
			running: Mutex::new(None),
		})
	}

	fn run(&self, py: Python<'_>) -> PyResult<LogIter> {
		py.allow_threads(|| {
			let gen_receiver = {
				let (gen_sender, gen_receiver) = mpsc::channel::<PacketData>();
				let _ = self.generator.lock().unwrap().insert(gen_sender);
				gen_receiver
			};
			let mut r = self.running.lock().unwrap();
			match &*r {
				Some(_) => return Err("already running"),
				None => (),
			};
			let (log_sender, log_receiver) = mpsc::channel::<ReceivedPacket>();
			let (s, buf_size, echo) = (self.server.clone(), self.buffer_size, self.echo);
			let t = thread::spawn(move || {
				if let Err(e) = client::run(s, buf_size, echo, gen_receiver, Some(log_sender)) {
					return Err(format!("client run failed: {e}"));
				}
				Ok(())
			});
			let _ = r.insert(t);
			Ok(LogIter::new(log_receiver))
		}).map_err(|e| PyException::new_err(e))
	}

	fn put(&self, py: Python<'_>, time: (i64, i64), size: usize) -> PyResult<()> {
		if size > self.buffer_size {
			return Err(PyValueError::new_err(
				"size too large, increase buffer_size"));
		} else if size < MIN_SIZE {
			return Err(PyValueError::new_err(
				format!("size smaller than minimum ({MIN_SIZE})")));
		}
		py.allow_threads(|| {
			let r = self.generator.lock().unwrap();
			if let Some(s) = r.as_ref() {
				let _ = s.send(PacketData {
					delay: TimeSpec::new(time.0, time.1),
					size,
				});
			}
			Ok(())
		})
	}

	fn close(&self, py: Python<'_>) {
		py.allow_threads(|| {
			let mut r = self.generator.lock().unwrap();
			r.take();
		});
	}

	fn join(&self, py: Python<'_>) -> PyResult<()> {
		py.allow_threads(|| {
			let mut r = self.running.lock().unwrap();
			if let None = &*r {
				return Err("not running");
			}
			let t = r.take().unwrap();
			match t.join() {
				Err(_) => Err("error in send thread"),
				Ok(_) => Ok(())
			}
		}).map_err(|e| PyException::new_err(e))
	}
}

impl LogIter {
	fn new(receiver: mpsc::Receiver<ReceivedPacket>) -> Self {
		LogIter {
			echo_receiver: Mutex::new(receiver),
		}
	}
}

#[pymethods]
impl LogIter {
	fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
		slf
	}

	fn __next__(&self, py: Python<'_>) -> Option<String> {
		py.allow_threads(|| {
			let guard = self.echo_receiver.lock().unwrap();
			match guard.recv() {
				Err(RecvError) => None,
				Ok(record) => Some(format!("{record}")),
			}
		})
	}
}

#[pymodule(gil_used = false)]
fn luna_py(m: &Bound<'_, PyModule>) -> PyResult<()> {
	m.add_class::<Client>()?;
	m.add_class::<LogIter>()?;
	m.add("MIN_SIZE", MIN_SIZE)?;
    Ok(())
}
