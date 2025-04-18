use crate::{set_rt_prio, ReceivedPacket, ECHO_FLAG, MIN_SIZE};
use nix::{
	cmsg_space,
	errno::Errno,
	sys::{
		mman,
		resource,
		socket::{self, SockaddrLike, SockaddrStorage},
		time::TimeSpec
	}
};
use std::{
	io::{Error, ErrorKind, IoSlice, IoSliceMut},
	os::fd::{AsRawFd, OwnedFd},
	sync::{mpsc, Mutex}
};


pub struct Server {
	bind: SockaddrStorage,
	buf_size: usize,
	logger: Option<mpsc::Sender<ReceivedPacket>>,
	sock: Option<OwnedFd>,
}


pub struct CloseHandle {
	fd: Mutex<Option<i32>>
}


impl Server {
	pub fn new(
		bind_addr: SockaddrStorage, buf_size: usize,
		logger: Option<mpsc::Sender<ReceivedPacket>>)
		-> Self
	{
		Server {
			bind: bind_addr,
			buf_size,
			logger,
			sock: None,
		}
	}

	/// Bind the server to the configured address. If the port is 0 in
	/// the bind address passed to Server::new(), this is where the
	/// actual port is picked.
	pub fn bind(&mut self) -> Result<CloseHandle, Errno> {
		let sock = socket::socket(
			self.bind.family().unwrap(),
			socket::SockType::Datagram,
			socket::SockFlag::empty(),
			None
		)?;
		socket::setsockopt(&sock, socket::sockopt::ReceiveTimestampns, &true)?;
		socket::bind(sock.as_raw_fd(), &self.bind)?;
		self.bind = socket::getsockname::<SockaddrStorage>(sock.as_raw_fd())?;
		let handle = CloseHandle::new(sock.as_raw_fd());
		self.sock = Some(sock);
		Ok(handle)
	}

	/// If the server is bound to a port (after successful
	/// Server::bind()), return the socket address of the server
	/// socket.
	pub fn bound(&self) -> Option<&SockaddrStorage> {
		if self.sock.is_some() {
			Some(&self.bind)
		} else {
			None
		}
	}

	pub fn run(&self) -> Result<(), Box<dyn std::error::Error>> {
		let fd = if let Some(sock) = self.sock.as_ref() {
			sock.as_raw_fd()
		} else {
			return Err(Box::new(Error::new(ErrorKind::NotConnected, "socket not bound")));
		};

		let flags = socket::MsgFlags::empty();
		let mut buffer = vec![0u8; self.buf_size];
		let mut cmsgspace = cmsg_space!(TimeSpec);
		let mut iov = [IoSliceMut::new(&mut buffer)];

		if self.logger.is_none() {
			println!("{}", ReceivedPacket::header());
		}

		crate::accept_noperm!(
			crate::with_capability(
				|| set_rt_prio(20),
				caps::Capability::CAP_SYS_NICE),
			"no permission to set realtime priority");

		// Prevent swapping, if possible. Needs to be done as late as
		// possible so all allocations needed for the loop are covered
		// with MCL_CURRENT.
		crate::accept_noperm!(
			crate::with_capability(
				|| mman::mlockall(mman::MlockAllFlags::MCL_CURRENT),
				caps::Capability::CAP_IPC_LOCK),
			"no permission to lock memory");

		caps::clear(None, caps::CapSet::Effective)?;
		caps::clear(None, caps::CapSet::Permitted)?;

		let rusage_pre = resource::getrusage(resource::UsageWho::RUSAGE_THREAD)?;

		loop {
			let r = socket::recvmsg::<socket::SockaddrStorage>(fd, &mut iov, Some(&mut cmsgspace), flags)?;
			if r.bytes == 0 {
				// server socket has been closed
				break;
			}
			let data = r.iovs().next().unwrap();

			// send echo if requested
			if r.bytes >= MIN_SIZE && 0 != (data[20] & ECHO_FLAG) {
				let iov = [IoSlice::new(data)];
				socket::sendmsg(fd, &iov, &[], flags, r.address.as_ref())?;
			}

			if let Ok(recv) = ReceivedPacket::try_from(r) {
				if let Some(sender) = &self.logger {
					if let Err(_) = sender.send(recv) {
						// receiver hung up, no point in listening
						break;
					}
				} else {
					println!("{recv}");
				}
			}
		}
		let rusage_post = resource::getrusage(resource::UsageWho::RUSAGE_THREAD)?;
		eprintln!("server shutting down");
		eprintln!(
			"major page faults: {}, minor page faults: {}",
			rusage_post.major_page_faults() - rusage_pre.major_page_faults(),
			rusage_post.minor_page_faults() - rusage_pre.minor_page_faults()
		);
		Ok(())
	}
}


impl CloseHandle {
	pub fn new(fd: i32) -> Self {
		CloseHandle {
			fd: Mutex::new(Some(fd))
		}
	}

	pub fn close(&self) -> Result<(), Errno> {
		let mut f = self.fd.lock().unwrap();
		match &f.take() {
			None => Ok(()),
			Some(fd) => match socket::shutdown(*fd, socket::Shutdown::Both).err() {
				None => Ok(()),
				Some(Errno::ENOTCONN) => Ok(()),
				Some(e) => return Err(e),
			}
		}
	}
}
