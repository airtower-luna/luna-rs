use std::{net::{IpAddr, SocketAddr, SocketAddrV4, SocketAddrV6, ToSocketAddrs}, sync::{mpsc::{self, RecvError}, Mutex}, thread};

use luna_rs::{client, server, PacketData, ReceivedPacket, MIN_SIZE};
use nix::{errno::Errno, sys::{socket::{self, SockaddrStorage}, time::TimeSpec}};
use pyo3::{exceptions::{PyException, PyOSError, PyValueError}, prelude::*};


#[pyclass(frozen, module = "luna_py")]
struct Client {
	server: SocketAddr,
	#[pyo3(get)]
	buffer_size: usize,
	#[pyo3(get)]
	echo: bool,
	generator: Mutex<Option<mpsc::Sender<PacketData>>>,
	running: Mutex<Option<thread::JoinHandle<Result<(), String>>>>,
}

#[pyclass(frozen, module = "luna_py")]
struct Server {
	bind: Mutex<SockaddrStorage>,
	#[pyo3(get)]
	buffer_size: usize,
	fd: Mutex<Option<i32>>,
}

#[pyclass(frozen, module = "luna_py")]
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

	#[getter]
	fn server(&self) -> String {
		format!("{}", self.server)
	}

	fn start(&self, py: Python<'_>) -> PyResult<LogIter> {
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

	#[getter]
	fn running(&self, py: Python<'_>) -> bool {
		py.allow_threads(|| {
			let r = self.running.lock().unwrap();
			match &*r {
				None => false,
				Some(t) => !t.is_finished(),
			}
		})
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


#[pymethods]
impl Server {
	#[new]
	#[pyo3(signature = (bind, port=7800, buffer_size=1500))]
	fn new(bind: &str, port: u16, buffer_size: usize) -> PyResult<Self> {
		let bind_ip: IpAddr = match bind.parse() {
			Ok(i) => i,
			Err(e) => { return Err(PyValueError::new_err(e)); },
		};
		let bind_addr = match bind_ip {
			IpAddr::V6(i) => SockaddrStorage::from(SocketAddrV6::new(i, port, 0, 0)),
			IpAddr::V4(i) => SockaddrStorage::from(SocketAddrV4::new(i, port)),
		};
		Ok(Server {
			bind: Mutex::new(bind_addr),
			buffer_size,
			fd: Mutex::new(None),
		})
	}

	pub fn start(&self, py: Python<'_>) -> PyResult<String> {
		py.allow_threads(|| {
			let (s, fd) = {
				let mut b = self.bind.lock().unwrap();
				let mut srv = server::Server::new(*b, self.buffer_size);
				if let Err(e) = srv.bind().map_err(|e| e.to_string()) {
					return Err(e)
				}
				// address the server is *actually* bound to
				*b = srv.bound().unwrap().clone();
				let fd = srv.fd();
				thread::spawn(move || srv.run().unwrap());
				(format!("{}", b), fd)
			};
			let mut f = self.fd.lock().unwrap();
			*f = fd;
			Ok(s)
		}).map_err(|e| PyException::new_err(e))
	}

	pub fn stop(&self, py: Python<'_>) -> PyResult<()> {
		py.allow_threads(|| {
			let f = self.fd.lock().unwrap();
			match &*f {
				None => (),
				Some(fd) => match socket::shutdown(
					*fd, socket::Shutdown::Both).err()
				{
					None => (),
					Some(Errno::ENOTCONN) => (),
					Some(e) => { return Err(e) },
				}
			}
			Ok(())
		}).map_err(|e| PyOSError::new_err(e.desc()))
	}
}


#[pymodule(gil_used = false)]
fn luna_py(m: &Bound<'_, PyModule>) -> PyResult<()> {
	m.add("MIN_SIZE", MIN_SIZE)?;
	m.add_class::<Client>()?;
	m.add_class::<Server>()?;
	m.add_class::<LogIter>()?;
    Ok(())
}
