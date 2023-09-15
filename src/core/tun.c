#include "lwip/err.h"
#include "lwip/ip4_addr.h"
#include "lwip/netif.h"
#include "lwip/priv/tcp_priv.h"
#include "lwip/tcp.h"
#include "lwip/tun.h"

void tun_init() {
  netif_init();
  tcp_init();
}

err_t tun_device_tcp_accept(void *arg, struct tcp_pcb *newpcb, err_t err) {
  if (err != ERR_OK) {
    return err;
  }
  tun_new_connection_callback_t* callback = (tun_new_connection_callback_t *)arg;
  return callback->fn(callback->arg, newpcb, err);
}

struct tcp_pcb* tun_device_has_new_tcp_connection(struct netif *netif, struct tcp_hdr *tcp_hdr, const ip_addr_t *dst_ip, const ip_addr_t *src_ip) {
  struct tcp_pcb* conn;
  err_t err;

  conn = tcp_new();

  tcp_arg(conn, netif->state);

  err = tcp_bind(conn, dst_ip, tcp_hdr->dest);

  conn = tcp_listen(conn);

  tcp_accept(conn, tun_device_tcp_accept);

  return conn;
}

err_t tun_device_output(struct netif *netif, struct pbuf *p,
       const ip4_addr_t *ipaddr) {
  return ERR_OK;
}

err_t tun_netif_init(struct netif *netif) {
  netif->has_new_tcp_connection_fn = tun_device_has_new_tcp_connection;
  netif->output = tun_device_output;
  return ERR_OK;
}

struct netif* tun_netif_new(u32_t ip_addr, u32_t netmask, u32_t gw_addr, tun_new_connection_callback_t *callback)
{
  struct netif *netif = mem_malloc(sizeof(struct netif));

  ip4_addr_t ip_addr_t = { .addr = ip_addr };
  ip4_addr_t netmask_t = { .addr = netmask };
  ip4_addr_t gw_addr_t = { .addr = gw_addr };

  netif_add(netif, &ip_addr_t, &netmask_t, &gw_addr_t, (void*)callback, tun_netif_init, NULL);
  netif_set_link_up(netif);
  netif_set_up(netif);

  return netif;
}
