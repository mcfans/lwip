#include "lwip/err.h"
#include "lwip/tcp.h"

struct tun_new_connection_callback {
  err_t (*fn)(void *arg, struct tcp_pcb *newpcb, err_t err);

  void *arg;
};

typedef struct tun_new_connection_callback tun_new_connection_callback_t;

struct netif* tun_netif_new(u32_t ip_addr, u32_t netmask, u32_t gw_addr, tun_new_connection_callback_t *callback);

void tun_init();
