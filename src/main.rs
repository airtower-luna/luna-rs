use luna_rs::{client, server};
use clap::{Parser, Subcommand};
use nix::sys::socket::SockaddrStorage;
use std::net::{IpAddr, SocketAddr, SocketAddrV4, SocketAddrV6, ToSocketAddrs};


#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
	/// size of the send or receive buffer, larger packets cannot be
	/// sent, larger incoming packets will be truncated
	#[arg(short, long, default_value_t = 1500)]
	buffer_size: usize,
	#[command(subcommand)]
	command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
	Client {
		/// server to send to
		#[arg(short, long, default_value = "localhost:7800")]
		server: String,
		/// request packet echo from server
		#[arg(short, long, default_value_t = false)]
		echo: bool,
	},
	Server {
		/// port to listen on
		#[arg(short, long, default_value_t = 7800)]
		port: u16,
		/// local address to bind to for listening
		#[arg(short, long, default_value = "::")]
		bind: IpAddr,
	},
}


fn main() -> Result<(), Box<dyn std::error::Error>> {
	let args = Args::parse();
	#[cfg(debug_assertions)]
	eprintln!("{args:?}");
	match args.command {
		Commands::Client { server, echo} => {
			let server_addr: SocketAddr = server
				.to_socket_addrs()
				.expect("cannot parse server address")
				.next().expect("no address");
			client::run(server_addr, args.buffer_size, echo)?;
		},
		Commands::Server { port, bind } => {
			let bind_addr: SockaddrStorage = if bind.is_ipv6() {
				let s = format!("[{}]:{}", bind, port);
				SockaddrStorage::from(s.parse::<SocketAddrV6>()?)
			} else {
				let s = format!("{}:{}", bind, port);
				SockaddrStorage::from(s.parse::<SocketAddrV4>()?)
			};
			server::run(bind_addr, args.buffer_size)?;
		},
	}
	Result::Ok(())
}
