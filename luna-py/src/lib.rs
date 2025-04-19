use std::{
	net::{IpAddr, SocketAddr, SocketAddrV4, SocketAddrV6, ToSocketAddrs},
	sync::{mpsc::{self, RecvError}, Mutex},
	thread
};

use luna_rs::{client, server, PacketData, ReceivedPacket, MIN_SIZE};
use nix::{errno::Errno, sys::{socket::SockaddrStorage, time::TimeSpec}};
use pyo3::{
	exceptions::{PyException, PyOSError, PyValueError},
	prelude::*,
	sync::GILOnceCell,
	types::{PyTraceback, PyType}
};


fn timespec_to_decimal<'py>(
	py: Python<'py>, time: &TimeSpec)
	-> PyResult<Bound<'py, PyAny>>
{
	static DECIMAL: GILOnceCell<Py<PyType>> = GILOnceCell::new();
	DECIMAL.import(py, "decimal", "Decimal")?
		.call1((format!("{}.{}", time.tv_sec(), time.tv_nsec()),))
}


#[pyclass(frozen, module = "luna")]
struct PacketRecord {
	packet: ReceivedPacket
}

#[pymethods]
impl PacketRecord {
	/// Source address of the packet. For echo packets received by the
	/// client this will be the server.
	#[getter]
	fn source(&self) -> String {
		format!("{}", self.packet.source)
	}

	/// Receive timestamp of the packet as reported by the kernel,
	/// decimal.Decimal in seconds.
	#[getter]
	fn receive_time<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
		timespec_to_decimal(py, &self.packet.receive_time)
	}

	/// Size of the packet (bytes).
	#[getter]
	fn size(&self) -> usize {
		self.packet.size
	}

	/// Sequence number of the packet.
	#[getter]
	fn sequence(&self) -> u32 {
		self.packet.sequence
	}

	/// Send timestamp recorded in the packet, as decimal.Decimal in
	/// seconds.
	#[getter]
	fn timestamp<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
		timespec_to_decimal(py, &self.packet.timestamp)
	}

	fn __str__(&self) -> String {
		format!("{}", self.packet)
	}

	fn __repr__(&self) -> String {
		format!("<luna.PacketRecord: {:?}>", self.packet)
	}
}


#[pyclass(frozen, module = "luna")]
struct Client {
	server: SocketAddr,
	#[pyo3(get)]
	buffer_size: usize,
	#[pyo3(get)]
	echo: bool,
	generator: Mutex<Option<mpsc::Sender<PacketData>>>,
	running: Mutex<Option<thread::JoinHandle<Result<(), String>>>>,
	log: Mutex<Option<mpsc::Receiver<ReceivedPacket>>>,
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
			log: Mutex::new(None),
		})
	}

	#[getter]
	fn server(&self) -> String {
		format!("{}", self.server)
	}

	fn start(&self, py: Python<'_>) -> PyResult<()> {
		py.allow_threads(|| {
			let gen_receiver = {
				let (gen_sender, gen_receiver) = mpsc::channel::<PacketData>();
				let _ = self.generator.lock().unwrap().insert(gen_sender);
				gen_receiver
			};
			let log_receiver = {
				let mut r = self.running.lock().unwrap();
				match &*r {
					Some(_) => return Err("already running"),
					None => (),
				};
				let (log_sender, log_receiver) = mpsc::channel::<ReceivedPacket>();
				let (s, buf_size, echo) = (self.server.clone(), self.buffer_size, self.echo);
				let t = thread::spawn(move || {
					if let Err(e) = client::run(
						s, buf_size, echo, gen_receiver, None, Some(log_sender))
					{
						return Err(format!("client run failed: {e}"));
					}
					Ok(())
				});
				*r = Some(t);
				log_receiver
			};
			{
				let mut l = self.log.lock().unwrap();
				*l = Some(log_receiver);
			}
			Ok(())
		}).map_err(|e| PyException::new_err(e))
	}

	fn put(&self, py: Python<'_>, delay: (i64, i64), size: usize) -> PyResult<()> {
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
					delay: TimeSpec::new(delay.0, delay.1),
					size,
				});
				Ok(())
			} else {
				Err("client is not running")
			}
		}).map_err(|e| PyException::new_err(e))
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
			match r.take().map(|t| t.join()) {
				None => Ok(()),
				Some(e) => e
					.map(|_| ())
					.map_err(|_| "panic in client thread")
			}
		}).map_err(|e| PyException::new_err(e))
	}

	fn __enter__<'py>(
		slf: PyRef<'py, Self>, py: Python<'py>)
		-> PyResult<PyRef<'py, Self>>
	{
		slf.start(py)?;
		Ok(slf)
	}

	fn __exit__<'py>(
		slf: PyRef<'py, Self>, py: Python<'py>,
		_exception_type: Option<&Bound<'py, PyType>>,
		_exception_value: Option<&Bound<'py, PyException>>,
		_traceback: Option<&Bound<'py, PyTraceback>>)
		-> PyResult<bool>
	{
		slf.close(py);
		slf.join(py)?;
		Ok(false)
	}

	fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
		slf
	}

	fn __next__(&self, py: Python<'_>) -> Option<PacketRecord> {
		py.allow_threads(|| {
			let guard = self.log.lock().unwrap();
			match guard.as_ref()
				.map(|r| r.recv())
				.unwrap_or(Err(RecvError))
			{
				Err(RecvError) => None,
				Ok(record) => Some(PacketRecord {packet: record}),
			}
		})
	}
}


#[pyclass(frozen, module = "luna")]
struct Server {
	bind: Mutex<SockaddrStorage>,
	#[pyo3(get)]
	buffer_size: usize,
	handle: Mutex<Option<server::CloseHandle>>,
	running: Mutex<Option<thread::JoinHandle<Result<(), String>>>>,
	log: Mutex<Option<mpsc::Receiver<ReceivedPacket>>>,
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
			handle: Mutex::new(None),
			running: Mutex::new(None),
			log: Mutex::new(None),
		})
	}

	pub fn start(&self, py: Python<'_>) -> PyResult<()> {
		py.allow_threads(|| {
			{
				let r = self.running.lock().unwrap();
				match *r {
					Some(_) => return Err(Errno::EISCONN),
					None => (),
				}
			}
			let (ch, jh, logger) = {
				let (log_sender, logger) = mpsc::channel();
				let mut b = self.bind.lock().unwrap();
				let mut srv = server::Server::new(*b, self.buffer_size, Some(log_sender));
				let server_handle = srv.bind()?;
				// address the server is *actually* bound to
				*b = srv.bound().unwrap().clone();
				let jh = thread::spawn(move || srv.run().map_err(|e| e.to_string()));
				(server_handle, jh, logger)
			};
			{
				let mut h = self.handle.lock().unwrap();
				*h = Some(ch);
			}
			{
				let mut j = self.running.lock().unwrap();
				*j = Some(jh);
			}
			{
				let mut l = self.log.lock().unwrap();
				*l = Some(logger);
			}
			Ok(())
		}).map_err(|e: Errno| PyOSError::new_err(e.desc()))
	}

	#[getter]
	pub fn bind(&self, py: Python<'_>) -> String {
		py.allow_threads(|| {
			let b = self.bind.lock().unwrap();
			format!("{}", b)
		})
	}

	pub fn stop(&self, py: Python<'_>) -> PyResult<()> {
		py.allow_threads(|| {
			let mut h = self.handle.lock().unwrap();
			match h.take() {
				None => Ok(()),
				Some(handle) => handle.close()
			}
		}).map_err(|e| PyOSError::new_err(e.desc()))
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
			match r.take().map(|t| t.join()) {
				None => Ok(()),
				Some(e) => e
					.map(|_| ())
					.map_err(|_| "panic in server thread")
			}
		}).map_err(|e| PyException::new_err(e))
	}

	fn __enter__<'py>(
		slf: PyRef<'py, Self>, py: Python<'py>)
		-> PyResult<PyRef<'py, Self>>
	{
		slf.start(py)?;
		Ok(slf)
	}

	fn __exit__<'py>(
		slf: PyRef<'py, Self>, py: Python<'py>,
		_exception_type: Option<&Bound<'py, PyType>>,
		_exception_value: Option<&Bound<'py, PyException>>,
		_traceback: Option<&Bound<'py, PyTraceback>>)
		-> PyResult<bool>
	{
		slf.stop(py)?;
		slf.join(py)?;
		Ok(false)
	}

	fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
		slf
	}

	fn __next__(&self, py: Python<'_>) -> Option<PacketRecord> {
		py.allow_threads(|| {
			let guard = self.log.lock().unwrap();
			match guard.as_ref()
				.map(|r| r.recv())
				.unwrap_or(Err(RecvError))
			{
				Err(RecvError) => None,
				Ok(record) => Some(PacketRecord {packet: record}),
			}
		})
	}
}


#[pymodule(gil_used = false)]
fn luna(m: &Bound<'_, PyModule>) -> PyResult<()> {
	m.add("MIN_SIZE", MIN_SIZE)?;
	m.add_class::<Client>()?;
	m.add_class::<Server>()?;
	m.add_class::<PacketRecord>()?;
    Ok(())
}
