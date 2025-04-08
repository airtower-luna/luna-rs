use std::{net::{SocketAddr, ToSocketAddrs}, sync::{mpsc::{self, RecvError}, Mutex}, thread};

use luna_rs::{client, generator::Generator, ReceivedPacket};
use pyo3::{exceptions::{PyException, PyValueError}, prelude::*};

#[pyclass(frozen)]
struct Client {
	server: SocketAddr,
	buffer_size: usize,
	echo: bool,
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
			running: Mutex::new(None)
		})
	}

	fn run(&self) -> PyResult<LogIter> {
		let mut r = self.running.lock().unwrap();
		match &*r {
			Some(_) => return Err(PyException::new_err("already running")),
			None => (),
		};
		let receiver = Generator::Vary.run();
		let (log_sender, log_receiver) = mpsc::channel::<ReceivedPacket>();
		let (s, buf_size, echo) = (self.server.clone(), self.buffer_size, self.echo);
		let t = thread::spawn(move || {
			if let Err(e) = client::run(s, buf_size, echo, receiver, Some(log_sender)) {
				return Err(format!("client run failed: {e}"));
			}
			Ok(())
		});
		let _ = r.insert(t);
		Ok(LogIter::new(log_receiver))
	}

	fn join(&self) -> PyResult<()> {
		let mut r = self.running.lock().unwrap();
		if let None = &*r {
			return Err(PyException::new_err("not running"));
		}
		let t = r.take().unwrap();
		match t.join() {
			Err(_) => Err(PyException::new_err("error in send thread")),
			Ok(_) => Ok(())
		}
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

	fn __next__(&self) -> Option<String> {
		let guard = self.echo_receiver.lock().unwrap();
		match guard.recv() {
			Err(RecvError) => None,
			Ok(record) => Some(format!("{record}")),
		}
	}
}

#[pymodule(gil_used = false)]
fn luna_py(m: &Bound<'_, PyModule>) -> PyResult<()> {
	m.add_class::<Client>()?;
	m.add_class::<LogIter>()?;
    Ok(())
}
