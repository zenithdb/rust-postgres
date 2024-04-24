use crate::config::Host;
use crate::keepalive::KeepaliveConfig;
use crate::{Error, Socket, SocketAddr};
use socket2::{SockRef, TcpKeepalive};
use std::future::Future;
use std::io;
use std::time::Duration;
#[cfg(unix)]
use tokio::net::UnixStream;
use tokio::net::{self, TcpStream};
use tokio::time;

pub(crate) async fn connect_socket_addr(
    host: &Host,
    port: u16,
    socket: SocketAddr,
    connect_timeout: Option<Duration>,
    keepalive_config: Option<&KeepaliveConfig>,
) -> Result<Socket, Error> {
    match socket {
        SocketAddr::Tcp(socket) => {
            let stream = connect_with_timeout(TcpStream::connect(socket), connect_timeout).await?;

            stream.set_nodelay(true).map_err(Error::connect)?;
            if let Some(keepalive_config) = keepalive_config {
                SockRef::from(&stream)
                    .set_tcp_keepalive(&TcpKeepalive::from(keepalive_config))
                    .map_err(Error::connect)?;
            }
            Ok(Socket::new_tcp(stream))
        }
        SocketAddr::Unix => match host {
            Host::Tcp(_) => unreachable!(),
            Host::Unix(path) => {
                let path = path.join(format!(".s.PGSQL.{}", port));
                let socket =
                    connect_with_timeout(UnixStream::connect(path), connect_timeout).await?;
                Ok(Socket::new_unix(socket))
            }
        },
    }
}

pub(crate) async fn connect_socket(
    host: &Host,
    port: u16,
    connect_timeout: Option<Duration>,
    keepalive_config: Option<&KeepaliveConfig>,
) -> Result<Socket, Error> {
    match host {
        Host::Tcp(host) => {
            let addrs = net::lookup_host((&**host, port))
                .await
                .map_err(Error::connect)?;

            let mut last_err = None;

            for addr in addrs {
                let stream =
                    match connect_with_timeout(TcpStream::connect(addr), connect_timeout).await {
                        Ok(stream) => stream,
                        Err(e) => {
                            last_err = Some(e);
                            continue;
                        }
                    };

                stream.set_nodelay(true).map_err(Error::connect)?;
                if let Some(keepalive_config) = keepalive_config {
                    SockRef::from(&stream)
                        .set_tcp_keepalive(&TcpKeepalive::from(keepalive_config))
                        .map_err(Error::connect)?;
                }

                return Ok(Socket::new_tcp(stream));
            }

            Err(last_err.unwrap_or_else(|| {
                Error::connect(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "could not resolve any addresses",
                ))
            }))
        }
        #[cfg(unix)]
        Host::Unix(path) => {
            let path = path.join(format!(".s.PGSQL.{}", port));
            let socket = connect_with_timeout(UnixStream::connect(path), connect_timeout).await?;
            Ok(Socket::new_unix(socket))
        }
    }
}

async fn connect_with_timeout<F, T>(connect: F, timeout: Option<Duration>) -> Result<T, Error>
where
    F: Future<Output = io::Result<T>>,
{
    match timeout {
        Some(timeout) => match time::timeout(timeout, connect).await {
            Ok(Ok(socket)) => Ok(socket),
            Ok(Err(e)) => Err(Error::connect(e)),
            Err(_) => Err(Error::connect(io::Error::new(
                io::ErrorKind::TimedOut,
                "connection timed out",
            ))),
        },
        None => match connect.await {
            Ok(socket) => Ok(socket),
            Err(e) => Err(Error::connect(e)),
        },
    }
}
