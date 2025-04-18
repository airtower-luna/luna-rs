use nix::{libc::timespec, sys::{socket, time::TimeSpec}};
use core::fmt;
use std::{fmt::{Display, Formatter}, io::{Error, ErrorKind}};

pub const ECHO_FLAG: u8 = 1;
pub const MIN_SIZE: usize = size_of::<u32>() + size_of::<timespec>() + size_of::<u8>();


pub mod generator;
pub mod client;
pub mod server;


/// Read an int of the given format from a byte slice. Will panic if
/// the slice does not contain enough bytes, check before call.
macro_rules! parse_int {
	($data:expr, $t:ty) => {{
		let (b, rest) = $data.split_at(size_of::<$t>());
		let parsed = <$t>::from_be_bytes(b.try_into().unwrap());
		(parsed, rest)
	}};
}


macro_rules! accept_noperm {
	($call:expr, $warn:literal) => {{
		if let Err(e) = $call {
			match e.downcast::<Error>() {
				Ok(e) => {
					if e.kind() == ErrorKind::PermissionDenied {
						eprintln!("{}: {}", $warn, e);
					} else {
						return Err(e);
					}
				},
				Err(e) => { return Err(e); }
			}
		}
	}};
}
pub(crate) use accept_noperm;


#[derive(Debug, PartialEq, Eq)]
pub struct PacketData {
	pub delay: TimeSpec,
	pub size: usize,
}


/// Add the given capability to the effective set, run the given
/// function, drop the capability from the effective set.
pub fn with_capability
	<T, E: std::error::Error + 'static, U: FnOnce() -> Result<T, E>>
	(func: U, cap: caps::Capability)
	 -> Result<T, Box<dyn std::error::Error>>
{
	if caps::has_cap(None, caps::CapSet::Permitted, cap)?
	{
		caps::raise(None, caps::CapSet::Effective, cap)?;
	} else {
		return Err(Box::new(Error::new(
			ErrorKind::PermissionDenied,
			format!("capability {cap} not in permitted set"))));
	}
	let ret = func();
	caps::drop(None, caps::CapSet::Effective, cap)?;
	match ret {
		Ok(r) => Ok(r),
		Err(e) => Err(Box::new(e)),
	}
}


/// Enable realtime scheduling for the current thread. The offset is
/// the priority relative to the minimum realtime priority. Requires
/// CAP_SYS_NICE capability in permitted set.
pub fn set_rt_prio(offset: i32) -> Result<(), Error> {
	let min_rt_prio = unsafe {
		libc::sched_get_priority_min(libc::SCHED_RR)
	};
	let max_rt_prio = unsafe {
		libc::sched_get_priority_max(libc::SCHED_RR)
	};
	#[cfg(not(target_env = "musl"))]
	let sparam = libc::sched_param {
		sched_priority: max_rt_prio.min(min_rt_prio + offset),
	};
	#[cfg(target_env = "musl")]
	let sparam = libc::sched_param {
		sched_priority: max_rt_prio.min(min_rt_prio + offset),
		// the following four values are required with musl, but
		// shouldn't matter because we use SCHED_RR, not
		// SCHED_SPORADIC ("ss" is short for "sporadic server")
		sched_ss_low_priority: 0,
		sched_ss_repl_period: timespec { tv_sec: 0, tv_nsec: 0 },
		sched_ss_init_budget: timespec { tv_sec: 0, tv_nsec: 0 },
		sched_ss_max_repl: 0,
	};
	let ret = unsafe {
		let thread_id = libc::pthread_self();
		libc::pthread_setschedparam(thread_id, libc::SCHED_RR, &sparam)
	};
	if ret != 0 {
		Err(Error::from_raw_os_error(ret))
	} else {
		Ok(())
	}
}


#[derive(Debug, PartialEq, Eq)]
pub struct ReceivedPacket {
	/// where the packet was received from (client on the server side,
	/// server for echo packets received by the client)
	pub source: socket::SockaddrStorage,
	/// time the packet was received
	pub receive_time: TimeSpec,
	/// size of the packet
	pub size: usize,
	/// sequence number recorded in the packet
	pub sequence: u32,
	/// send timestamp as recorded in the packet
	pub timestamp: TimeSpec,
	/// flags recorded in the packet
	pub flags: u8,
}


impl TryFrom<socket::RecvMsg<'_, '_, socket::SockaddrStorage>> for ReceivedPacket {
	type Error = std::io::Error;

	fn try_from(r: socket::RecvMsg<'_, '_, socket::SockaddrStorage>) -> Result<Self, Self::Error> {
		let data = r.iovs().next().unwrap();
		if r.bytes < MIN_SIZE {
			return Err(Error::new(ErrorKind::InvalidData, "packet too small"));
		}
		let source = r.address
			.ok_or_else(|| Error::new(ErrorKind::InvalidData, "no source address"))?;
		let rtime = r.cmsgs()?
			.filter_map(|c| match c {
				socket::ControlMessageOwned::ScmTimestampns(t) => Some(t),
				_ => None
			})
			.next()
			.ok_or_else(|| Error::new(ErrorKind::InvalidData, "no receive time data"))?;

		let (seq, rest) = parse_int!(data, u32);
		let (sec, rest) = parse_int!(rest, i64);
		let (nsec, rest) = parse_int!(rest, i64);
		let stamp = TimeSpec::new(sec, nsec);

		Ok(ReceivedPacket {
			source,
			receive_time: rtime,
			size: r.bytes,
			sequence: seq,
			timestamp: stamp,
			flags: rest[0],
		})
	}
}


impl Display for ReceivedPacket {
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
		let (ip, port) = if let Some(a) = self.source.as_sockaddr_in6() {
			(format!("{}", a.ip()), a.port())
		} else { if let Some(a) = self.source.as_sockaddr_in() {
			(format!("{}", a.ip()), a.port())
		} else {
			return fmt::Result::Err(fmt::Error::default());
		}};
		write!(
			f, "{}.{:09}\t{}\t{}\t{}\t{}.{:09}\t{}",
			self.receive_time.tv_sec(), self.receive_time.tv_nsec(),
			ip, port, self.sequence,
			self.timestamp.tv_sec(), self.timestamp.tv_nsec(),
			self.size)
	}
}


impl ReceivedPacket {
	pub fn header() -> String {
		String::from("receive_time\tsource\tport\tsequence\ttimestamp\tsize")
	}
}


#[cfg(test)]
mod tests {
	use std::{
		collections::HashMap,
		net::{Ipv6Addr, SocketAddrV6, ToSocketAddrs},
		sync::mpsc::{self, RecvError},
		thread,
		time::Duration
	};

	use generator::Generator;
	use socket::SockaddrStorage;

	use super::*;

	/// Test the full cycle: Start a server, start a client with echo
	/// enabled, check that the client echo log reports the expected
	/// packets.
	#[test]
	fn full() -> Result<(), Box<dyn std::error::Error>> {
		let buf_size = 32;
		// address with 0 port to make the server pick a free one
		let bind_addr = SockaddrStorage::from("[::1]:0".parse::<SocketAddrV6>()?);
		let (server_log_sender, server_logger) = mpsc::channel();
		let mut srv = server::Server::new(
			bind_addr, buf_size, Some(server_log_sender));
		let server_handle = srv.bind()?;
		// address the server is *actually* bound to
		let bind_addr = srv.bound().unwrap().clone();
		let s = format!("{}", bind_addr);
		let sh = thread::spawn(move || srv.run().unwrap());

		let count = 200;
		let mut go: HashMap<String, String> = HashMap::new();
		go.insert("usec".to_string(), "30".to_string());
		go.insert("count".to_string(), format!("{count}"));
		let receiver = Generator::Default.run(go)?;
		let server_addr: std::net::SocketAddr = s.to_socket_addrs()
			.expect("cannot parse server address")
			.next().expect("no address");
		let (client_log_sender, client_logger) = mpsc::channel();
		let ct = thread::spawn(move || {
			client::run(
				server_addr, buf_size,
				true, receiver,
				Some(Duration::from_millis(50)), Some(client_log_sender)
			).map_err(|e| e.to_string())
		});

		// check that the server sees all packets
		let slh = thread::spawn(move || {
			for i in 0..count {
				let r = server_logger.recv().unwrap();
				assert_eq!(
					r.source.as_sockaddr_in6().unwrap().ip(),
					"::1".parse::<Ipv6Addr>().unwrap());
				assert_eq!(r.size, MIN_SIZE);
				assert_eq!(r.sequence, i);
			}
		});

		// check that the client sees all echoes
		for i in 0..count {
			let r = client_logger.recv()?;
			assert_eq!(r.source, bind_addr);
			assert_eq!(r.size, MIN_SIZE);
			assert_eq!(r.sequence, i);
		}
		assert_eq!(client_logger.recv(), Err(RecvError));

		server_handle.close()?;
		slh.join().unwrap();

		if let Err(e) = ct.join() {
			eprintln!("panic in client thread: {e:?}");
		};
		if let Err(e) = sh.join() {
			eprintln!("panic in server thread: {e:?}");
		};

		Ok(())
	}

	#[test]
	fn rt_priority() {
		let offset = 5;
		match with_capability(
			|| set_rt_prio(offset),
			caps::Capability::CAP_SYS_NICE)
		{
			Err(e) => assert_eq!(e.downcast::<Error>().unwrap().kind(), ErrorKind::PermissionDenied),
			Ok(_) => {
				if unsafe { libc::geteuid() } == 0 {
					panic!("Don't run tests as root!")
				}
				let min_rt_prio = unsafe {
					libc::sched_get_priority_min(libc::SCHED_RR)
				};
				#[cfg(not(target_env = "musl"))]
				let mut sparam = libc::sched_param {
					sched_priority: 0,
				};
				#[cfg(target_env = "musl")]
				let sparam = libc::sched_param {
					sched_priority: 0,
					sched_ss_low_priority: 0,
					sched_ss_repl_period: timespec { tv_sec: 0, tv_nsec: 0 },
					sched_ss_init_budget: timespec { tv_sec: 0, tv_nsec: 0 },
					sched_ss_max_repl: 0,
				};
				let mut policy: libc::c_int = 0;
				let ret = unsafe {
					let thread_id = libc::pthread_self();
					libc::pthread_getschedparam(
						thread_id, &mut policy, &mut sparam)
				};
				assert_eq!(ret, 0);
				assert_eq!(policy, libc::SCHED_RR);
				assert_eq!(sparam.sched_priority, min_rt_prio + offset);
				// capability has been dropped
				assert_eq!(
					caps::has_cap(
						None, caps::CapSet::Effective,
						caps::Capability::CAP_SYS_NICE).unwrap(),
					false);
			},
		}
	}
}
