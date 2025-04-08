use crate::{set_rt_prio, ReceivedPacket, ECHO_FLAG, MIN_SIZE};
use nix::{cmsg_space, sys::{mman, socket::{self, SockaddrLike, SockaddrStorage}, time::TimeSpec}};
use std::{io::{Error, ErrorKind, IoSlice, IoSliceMut}, os::fd::{AsRawFd, OwnedFd}};


pub struct Server {
	bind: SockaddrStorage,
	buf_size: usize,
	sock: Option<OwnedFd>,
}


impl Server {
	pub fn new(bind_addr: SockaddrStorage, buf_size: usize) -> Self {
		Server {
			bind: bind_addr,
			buf_size,
			sock: None,
		}
	}

	/// Bind the server to the configured address. If the port is 0 in
	/// the bind address passed to Server::new(), this is where the
	/// actual port is picked.
	pub fn bind(&mut self) -> Result<(), Box<dyn std::error::Error>> {
		let sock = socket::socket(
			self.bind.family().unwrap(),
			socket::SockType::Datagram,
			socket::SockFlag::empty(),
			None
		)?;
		socket::setsockopt(&sock, socket::sockopt::ReceiveTimestampns, &true)?;
		socket::bind(sock.as_raw_fd(), &self.bind)?;
		self.bind = socket::getsockname::<SockaddrStorage>(sock.as_raw_fd())?;
		self.sock = Some(sock);
		Ok(())
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
		// prevent swapping, if possible
		if let Err(e) = mman::mlockall(
			mman::MlockAllFlags::MCL_CURRENT
				| mman::MlockAllFlags::MCL_FUTURE) {
			eprintln!("could not lock memory: {}", e);
		}

		if let Err(err) = set_rt_prio(20) {
			eprintln!("could not set realtime priority: {}", err);
		}

		let flags = socket::MsgFlags::empty();
		let mut buffer = vec![0u8; self.buf_size];
		let mut cmsgspace = cmsg_space!(TimeSpec);
		let mut iov = [IoSliceMut::new(&mut buffer)];

		println!("{}", ReceivedPacket::header());
		loop {
			let r = socket::recvmsg::<socket::SockaddrStorage>(fd, &mut iov, Some(&mut cmsgspace), flags)?;
			let data = r.iovs().next().unwrap();

			// send echo if requested
			if r.bytes >= MIN_SIZE && 0 != (data[20] & ECHO_FLAG) {
				let iov = [IoSlice::new(data)];
				socket::sendmsg(fd, &iov, &[], flags, r.address.as_ref())?;
			}

			if let Ok(recv) = ReceivedPacket::try_from(r) {
				println!("{recv}");
			}
		}
	}
}


pub fn run(bind_addr: SockaddrStorage, buf_size: usize)
		   -> Result<(), Box<dyn std::error::Error>>
{
	let mut srv = Server::new(bind_addr, buf_size);
	srv.bind()?;
	srv.run()?;
	Ok(())
}
