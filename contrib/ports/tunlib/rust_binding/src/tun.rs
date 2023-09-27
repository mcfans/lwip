use crate::lwip_binding::{
    self, err_t, netif, netif_input, pbuf, pbuf_alloc, pbuf_layer_PBUF_RAW, pbuf_take,
    pbuf_type_PBUF_POOL, tcp_pcb, tcp_tcp_get_tcp_addrinfo, tun_device_callback, tun_netif_new,
};
use rayon::ThreadPool;
use std::net::{Ipv4Addr, SocketAddr};
use std::os::raw::c_void;

pub struct TunNetif {
    netif: *mut netif,
    context: *const NetIfContext,
}

struct NetIfContext {
    pipe: Box<dyn Pipe>,
    pool: std::sync::Arc<ThreadPool>,
    output: Option<Box<dyn Fn(&[u8]) -> ()>>,
}

pub trait Pipe {
    fn handle_new_connection(&self, conn: crate::tcp::TcpConnection, dst: SocketAddr);
}

extern "C" fn new_connection_callback(
    arg: *mut ::std::os::raw::c_void,
    newpcb: *mut tcp_pcb,
    err: err_t,
) -> err_t {
    println!("New Connection from lwip {:?}", newpcb);
    if err != crate::lwip_binding::err_enum_t_ERR_OK as err_t {
        return err;
    }
    let context = unsafe { (arg as *const NetIfContext).as_ref().unwrap() };

    let pool = &context.pool;

    let pcb_wrapper = PtrWrapper(newpcb);

    let socket_addr = pool.install(|| {
        let pcb_wrapper = pcb_wrapper;

        let mut remote_ip: crate::lwip_binding::ip4_addr_t = unsafe { std::mem::zeroed() };
        let mut remote_port: u16 = 0;
        let err = unsafe { tcp_tcp_get_tcp_addrinfo(pcb_wrapper.0, 1, &mut remote_ip, &mut remote_port) };

        if err != crate::lwip_binding::err_enum_t_ERR_OK as err_t {
            return Result::Err(err);
        }

        let addr: [u8; 4] = unsafe { std::mem::transmute(remote_ip) };

        let ip_addr = Ipv4Addr::new(addr[0], addr[1], addr[2], addr[3]);
        let socket_addr = SocketAddr::new(ip_addr.into(), remote_port);

        Ok(socket_addr)
    });

    if let Err(err) = socket_addr {
        return err;
    }

    let conn = crate::tcp::TcpConnection::new(newpcb, pool.clone());

    context.pipe.handle_new_connection(conn, socket_addr.unwrap());
    return crate::lwip_binding::err_enum_t_ERR_OK as err_t;
}

extern "C" fn output_data(
    arg: *mut ::std::os::raw::c_void,
    _: *mut netif,
    pbuf: *mut pbuf,
    _: *const crate::lwip_binding::ip4_addr_t,
) -> err_t {
    unsafe {
        let context = (arg as *const NetIfContext).as_ref().unwrap();
        if let Some(output_fn) = context.output.as_ref() {
            let data =
                std::slice::from_raw_parts((*pbuf).payload as *const u8, (*pbuf).len as usize);
            output_fn(data);
        }
    }

    return crate::lwip_binding::err_enum_t_ERR_OK as err_t;
}

pub(crate) struct PtrWrapper<T>(pub(crate) T);

unsafe impl<T> Send for PtrWrapper<T> {}
unsafe impl<T> Sync for PtrWrapper<T> {}

impl TunNetif {
    pub fn new(
        handle: tokio::runtime::Handle,
        ip_addr: Ipv4Addr,
        net_mask: Ipv4Addr,
        gateway: Ipv4Addr,
        pipe: Box<dyn Pipe>,
    ) -> TunNetif {
        unsafe {
            let ip_addr: u32 = std::mem::transmute(ip_addr.octets());
            let net_mask: u32 = std::mem::transmute(net_mask.octets());
            let gateway: u32 = std::mem::transmute(gateway.octets());

            lwip_binding::lwip_init();

            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(1)
                .thread_name(|i| format!("lwip-TCP-{}", i))
                .build()
                .unwrap();

            let arc_pool = std::sync::Arc::new(pool);

            let context = NetIfContext {
                pipe,
                output: None,
                pool: arc_pool.clone(),
            };

            let boxed = Box::new(context);
            let addr = Box::into_raw(boxed);
            let context_wrapper = PtrWrapper(addr as *const NetIfContext);

            let ptr_to_netif = arc_pool.install(|| {
                let wrapper = context_wrapper;
                let context_ptr = wrapper.0;

                let callback: crate::lwip_binding::tun_device_callback = tun_device_callback {
                    new_connection: Some(new_connection_callback),
                    output: Some(output_data),
                    arg: context_ptr as *mut c_void,
                };

                let boxed_callback = Box::new(callback);
                let addr_boxed_callback = Box::into_raw(boxed_callback);

                let ptr = tun_netif_new(
                    ip_addr.to_be(),
                    net_mask.to_be(),
                    gateway.to_be(),
                    addr_boxed_callback,
                );
                PtrWrapper(ptr)
            });

            let cloned_pool = arc_pool.clone();

            handle.spawn(async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_millis(500));
                loop {
                    interval.tick().await;
                    cloned_pool.install(|| {
                        crate::lwip_binding::sys_check_timeouts();
                    })
                }
            });

            TunNetif {
                netif: ptr_to_netif.0,
                context: addr,
            }
        }
    }

    pub fn input_data(&self, data: &[u8]) {
        unsafe {
            let pbuf = pbuf_alloc(pbuf_layer_PBUF_RAW, data.len() as u16, pbuf_type_PBUF_POOL);
            pbuf_take(pbuf, data.as_ptr() as *const c_void, data.len() as u16);

            let pbuf_wrapper = PtrWrapper(pbuf);
            let netif_wrapper = PtrWrapper(self.netif);

            (*self.context).pool.install(|| {
                let netif_wrapper = netif_wrapper;
                let pbuf_wrapper = pbuf_wrapper;

                netif_input(pbuf_wrapper.0, netif_wrapper.0);
            });
        }
    }

    pub fn set_output_fn(&mut self, output: Box<dyn Fn(&[u8]) -> ()>) {
        unsafe {
            (self.context as *mut NetIfContext).as_mut().unwrap().output = Some(output);
        }
    }
}

impl Drop for TunNetif {
    fn drop(&mut self) {
        unsafe {
            crate::lwip_binding::netif_remove(self.netif);
            _ = Box::from_raw(self.context as *mut NetIfContext);
        }
    }
}
