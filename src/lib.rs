use nix::{libc::timespec, sys::{socket, time::TimeSpec}};
use core::fmt;
use std::{fmt::{Display, Formatter}, io::{Error, ErrorKind}};

pub const ECHO_FLAG: u8 = 1;
pub const MIN_SIZE: usize = size_of::<u32>() + size_of::<timespec>() + size_of::<u8>();


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


pub struct PacketData {
	pub delay: TimeSpec,
	pub size: usize,
}


/// Enable realtime scheduling for the current thread. The offset is
/// the priority relative to the minimum realtime priority. Requires
/// CAP_SYS_NICE capability.
pub fn set_rt_prio(offset: i32) -> Result<(), Error> {
	let min_rt_prio = unsafe {
		libc::sched_get_priority_min(libc::SCHED_RR)
	};
	let max_rt_prio = unsafe {
		libc::sched_get_priority_max(libc::SCHED_RR)
	};
	let sparam = libc::sched_param {
		sched_priority: max_rt_prio.min(min_rt_prio + offset),
	};
	let ret = unsafe {
		libc::sched_setscheduler(0, libc::SCHED_RR, &sparam)
	};
	if ret < 0 {
		Err(Error::last_os_error())
	} else {
		Ok(())
	}
}


#[derive(Debug)]
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
