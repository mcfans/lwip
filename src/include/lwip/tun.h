#include "lwip/err.h"
#include "lwip/netif.h"
#include "lwip/tcp.h"

struct tun_device_callback {
  err_t (*new_connection)(void *arg, struct tcp_pcb *newpcb, err_t err);

  err_t (*output)(void *arg, struct netif *netif, struct pbuf *p, const ip4_addr_t *ipaddr);

  void *arg;
};

typedef struct tun_device_callback tun_device_callback_t;

struct netif* tun_netif_new(u32_t ip_addr, u32_t netmask, u32_t gw_addr, tun_device_callback_t *callback);

void tun_init();
