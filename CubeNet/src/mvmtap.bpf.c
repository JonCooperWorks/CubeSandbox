// SPDX-License-Identifier: GPL-2.0
/* Copyright (c) 2022 Cube Authors */
#include <vmlinux.h>
#include <bpf/bpf_core_read.h>
#include <bpf/bpf_endian.h>
#include <bpf/bpf_helpers.h>
#include <bpf/bpf_tracing.h>

#include "cubevs.h"
#include "l2l3.h"
#include "nat.h"
#include "icmp.h"
#include "jhash.h"
#include "map.h"
#include "skb.h"
#include "tcp.h"
#include "udp.h"

/*
 * Handle ARP request and send ARP reply
 * This function performs ARP proxy (ARP spoofing) to answer ARP requests
 * from Sandbox with the gateway MAC address.
 *
 * Returns:
 *   TC_ACT_SHOT - if the packet should be dropped
 *   >= 0        - if the packet was handled (ARP reply sent)
 */
static __always_inline int handle_arp(struct __sk_buff *skb, __u32 ifindex)
{
	union macaddr *macaddr, tmp_macaddr;
	struct ethhdr *eth;
	struct arphdr_eth *arp;
	void *data, *data_end;
	__u32 len, ip;
	long err;

	/* Pull ARP packet headers */
	len = sizeof(struct ethhdr) + sizeof(struct arphdr_eth);
	err = bpf_skb_pull_data(skb, len);
	if (err)
		return TC_ACT_SHOT;

	data = (void *)(__u64)skb->data;
	data_end = (void *)(__u64)skb->data_end;

	if (data + len > data_end)
		return TC_ACT_SHOT;

	eth = data;
	arp = (struct arphdr_eth *)(eth + 1);

	/* Only handle Ethernet/IPv4 ARP requests */
	/* clang-format off */
	if (arp->ar_hrd != bpf_htons(ARPHRD_ETHER) ||
	    arp->ar_pro != bpf_htons(ETH_P_IP) ||
	    arp->ar_hln != ETH_ALEN ||
	    arp->ar_pln != sizeof(__be32) ||
	    arp->ar_op != bpf_htons(ARPOP_REQUEST))
		return TC_ACT_SHOT;
	/* clang-format on */

	/* Build ARP reply */
	arp->ar_op = bpf_htons(ARPOP_REPLY);

	ip = arp->ar_sip;
	arp->ar_sip = arp->ar_tip;
	arp->ar_tip = ip;

	macaddr = (union macaddr *)arp->ar_sha;
	tmp_macaddr.p1 = macaddr->p1;
	tmp_macaddr.p2 = macaddr->p2;
	/* Use gateway MAC as the sender (ARP proxy) */
	macaddr->p1 = cubegw0_macaddr_p1;
	macaddr->p2 = cubegw0_macaddr_p2;
	macaddr = (union macaddr *)arp->ar_tha;
	macaddr->p1 = tmp_macaddr.p1;
	macaddr->p2 = tmp_macaddr.p2;

	/* Update Ethernet header */
	macaddr = (union macaddr *)eth->h_source;
	tmp_macaddr.p1 = macaddr->p1;
	tmp_macaddr.p2 = macaddr->p2;
	macaddr->p1 = cubegw0_macaddr_p1;
	macaddr->p2 = cubegw0_macaddr_p2;
	macaddr = (union macaddr *)eth->h_dest;
	macaddr->p1 = tmp_macaddr.p1;
	macaddr->p2 = tmp_macaddr.p2;

	/* Send the reply back to the same interface */
	return bpf_redirect(ifindex, 0);
}

static __always_inline bool should_do_nat(const struct iphdr *l3)
{
	__u16 frag_off;

	/* Support TCP, UDP, and ICMP */
	if (l3->protocol != IPPROTO_TCP && l3->protocol != IPPROTO_UDP && l3->protocol != IPPROTO_ICMP)
		return false;

	frag_off = l3->frag_off;
	if ((frag_off & IP_FLAG_MF) || (frag_off & IP_FRAG_OFF_MASK))
		return false;

	return true;
}

/*
 * Check egress network policy for a packet.
 *
 * Priority: allow_out > deny_out > default allow
 *
 *   1. If allow_out has an inner map for this ifindex and daddr matches,
 *      the packet is explicitly allowed (even if deny_out would match).
 *   2. If deny_out has an inner map for this ifindex and daddr matches,
 *      the packet is denied.
 *   3. Otherwise the packet is allowed.
 *
 * Returns true if the packet is allowed, false if denied.
 */
static __always_inline bool check_net_policy(__u32 ifindex, __u32 daddr)
{
	struct lpm_key key = { .prefixlen = 32, .ip = daddr };
	void *inner_map;

	/* Traffic to mvm_gateway_ip is internal (destined for cube-dev),
	 * skip network policy enforcement.
	 */
	if (daddr == mvm_gateway_ip)
		return true;

	/* allow_out takes precedence */
	inner_map = bpf_map_lookup_elem(&allow_out, &ifindex);
	if (inner_map) {
		if (bpf_map_lookup_elem(inner_map, &key))
			return true;
		/* allow_out map exists but daddr not in it,
		 * fall through to deny_out check.
		 */
	}

	/* check deny_out */
	inner_map = bpf_map_lookup_elem(&deny_out, &ifindex);
	if (inner_map) {
		if (bpf_map_lookup_elem(inner_map, &key))
			return false;
	}

	/* default: allow */
	return true;
}

static __always_inline struct snat_ip *pick_snat_ip_port(__u32 mvm_ip, const struct session_key *ekey,
							 __u16 *selected_port)
{
	static const int max_retries = 10;
	struct ingress_session isess = {
		.version = ekey->version,
		.vm_ip = ekey->src_ip,
		.vm_port = ekey->src_port,
	};
	struct session_key ikey = {};
	struct snat_ip *snat_ip;
	__u16 snat_port;
	__u32 index;
	int i;

	index = jhash_1word(mvm_ip, HASH_SEED) % MAX_SNAT_IPS;
	snat_ip = bpf_map_lookup_elem(&snat_iplist, &index);
	if (!snat_ip)
		return NULL;

	ikey.src_ip = ekey->dst_ip;
	ikey.dst_ip = snat_ip->ip;
	ikey.src_port = ekey->dst_port;
	ikey.version = 0;
	ikey.protocol = ekey->protocol;
	for (i = 0; i < max_retries; i++) {
		bpf_spin_lock(&snat_ip->lock);
		snat_port = snat_ip->max_port;
		if (snat_ip->max_port == 0xffff)
			snat_ip->max_port = MAX_PORT_START;
		else
			snat_ip->max_port++;
		bpf_spin_unlock(&snat_ip->lock);

		ikey.dst_port = bpf_htons(snat_port);
		/* update with BPF_NOEXIST to take the slot without race */
		if (!bpf_map_update_elem(&ingress_sessions, &ikey, &isess, BPF_NOEXIST)) {
			/* at this point, we have ingress session created */
			*selected_port = bpf_htons(snat_port);
			return snat_ip;
		}
	}

	return NULL;
}

static __always_inline void del_session(struct session_key *ekey, struct nat_session *sess)
{
	struct session_key ikey = {
		.src_ip = ekey->dst_ip,
		.dst_ip = sess->node_ip,
		.src_port = ekey->dst_port,
		.dst_port = sess->node_port,
		.version = 0,
		.protocol = ekey->protocol,
	};

	bpf_map_delete_elem(&egress_sessions, ekey);
	bpf_map_delete_elem(&ingress_sessions, &ikey);
}

static bool do_icmp_nat(struct __sk_buff *skb, struct mvm_meta *mvm_meta, __u32 *dst_ifindex)
{
	struct icmp_id_buff old_id = {}, new_id = {};
	__u32 old_csum, new_csum;
	struct session_key key = {};
	struct nat_session *sess;
	struct snat_ip *snat_ip;
	struct ethhdr *l2;
	struct iphdr *l3;
	struct icmphdr *l4;
	__u16 snat_id;
	__u64 now;
	bool ok;

	if (!__pull_headers_icmp(skb, &l2, &l3, &l4))
		return false;

	/* Only handle Echo Request outbound; drop other ICMP types */
	if (l4->type != ICMP_ECHO)
		return false;

	now = bpf_ktime_get_ns();
	/* Use ICMP identifier as the "port" identifier in the session key */
	key.src_ip = mvm_meta->ip;
	key.dst_ip = l3->daddr;
	key.src_port = l4->un.echo.id; /* identifier (network byte order) */
	key.dst_port = 0;
	key.version = mvm_meta->version;
	key.protocol = IPPROTO_ICMP;

	sess = bpf_map_lookup_elem(&egress_sessions, &key);
	if (sess) {
		update_icmp_session(IP_CT_DIR_ORIGINAL, sess, now);
		goto do_nat;
	}

	/* create new session */
	snat_ip = pick_snat_ip_port(mvm_meta->ip, &key, &snat_id);
	if (!snat_ip || !snat_ip->ip || !snat_id)
		return false;
	ok = create_icmp_sessions(&key, now, skb->ingress_ifindex, snat_ip, snat_id);
	if (!ok)
		return false;
	sess = bpf_map_lookup_elem(&egress_sessions, &key);
	if (!sess)
		return false;

do_nat:
	/* update ICMP identifier (no pseudo-header checksum for ICMP).
	 * bpf_csum_diff requires 4-byte aligned sizes, use icmp_id_buff.
	 */
	old_id.id = l4->un.echo.id;
	new_id.id = sess->node_port;
	old_csum = l4->checksum;
	new_csum = bpf_csum_diff((void *)&old_id, sizeof(old_id), (void *)&new_id, sizeof(new_id), ~old_csum);
	l4->checksum = csum_fold(new_csum) ?: 0xffff;
	l4->un.echo.id = sess->node_port;

	/* update L3 source address */
	rewrite_l3_addr(l3, &l3->saddr, sess->node_ip);

	/* update L2 */
	set_mac_pair(l2, nodenic_macaddr_p1, nodenic_macaddr_p2,
		     nodegw_macaddr_p1, nodegw_macaddr_p2);

	*dst_ifindex = sess->node_ifindex;
	return true;
}

static bool do_udp_nat(struct __sk_buff *skb, struct mvm_meta *mvm_meta, __u32 *dst_ifindex)
{
	__u32 old_csum, new_csum;
	struct csum_buff old_buff = {}, new_buff = {};
	struct session_key key = {};
	struct nat_session *sess;
	struct snat_ip *snat_ip;
	struct ethhdr *l2;
	struct iphdr *l3;
	struct udphdr *l4;
	__u16 snat_port;
	__u64 now;
	bool ok;

	if (!__pull_headers_udp(skb, &l2, &l3, &l4))
		return false;

	now = bpf_ktime_get_ns();
	key.src_ip = mvm_meta->ip;
	key.dst_ip = l3->daddr;
	key.src_port = l4->source;
	key.dst_port = l4->dest;
	key.version = mvm_meta->version;
	key.protocol = IPPROTO_UDP;

	sess = bpf_map_lookup_elem(&egress_sessions, &key);
	if (sess) {
		update_udp_session(IP_CT_DIR_ORIGINAL, sess, now);
		goto do_nat;
	}

	/* create new session */
	snat_ip = pick_snat_ip_port(mvm_meta->ip, &key, &snat_port);
	if (!snat_ip || !snat_ip->ip || !snat_port)
		return false;
	ok = create_udp_sessions(&key, now, skb->ingress_ifindex, snat_ip, snat_port);
	if (!ok)
		return false;
	sess = bpf_map_lookup_elem(&egress_sessions, &key);
	if (!sess)
		return false;

do_nat:
	/* update L4 */
	old_buff.addr = l3->saddr;
	old_buff.port = l4->source;
	new_buff.addr = sess->node_ip;
	new_buff.port = sess->node_port;
	old_csum = l4->check;
	if (old_csum) {
		new_csum = bpf_csum_diff((void *)&old_buff, sizeof(old_buff), (void *)&new_buff, sizeof(new_buff),
					 ~old_csum);
		l4->check = csum_fold(new_csum) ?: 0xffff;
	}
	l4->source = sess->node_port;

	/* update L3 */
	rewrite_l3_addr(l3, &l3->saddr, sess->node_ip);

	/* update L2 */
	set_mac_pair(l2, nodenic_macaddr_p1, nodenic_macaddr_p2,
		     nodegw_macaddr_p1, nodegw_macaddr_p2);

	*dst_ifindex = sess->node_ifindex;
	return true;
}

static bool do_tcp_nat(struct __sk_buff *skb, struct mvm_meta *mvm_meta, __u32 *dst_ifindex)
{
	__u32 old_csum, new_csum;
	struct csum_buff old_buff = {}, new_buff = {};
	struct session_key key = {};
	struct nat_session *sess;
	struct snat_ip *snat_ip;
	bool syn, ack, fin, rst;
	struct ethhdr *l2;
	struct iphdr *l3;
	struct tcphdr *l4;
	__u16 snat_port;
	__u64 now;
	bool ok;

	if (!__pull_headers(skb, &l2, &l3, &l4))
		return false;

	now = bpf_ktime_get_ns();
	syn = l4->syn;
	ack = l4->ack;
	fin = l4->fin;
	rst = l4->rst;
	key.src_ip = mvm_meta->ip;
	key.dst_ip = l3->daddr;
	key.src_port = l4->source;
	key.dst_port = l4->dest;
	key.version = mvm_meta->version;
	key.protocol = l3->protocol;
	if (syn && !ack && !fin && !rst) {
		/* retransmission */
		sess = bpf_map_lookup_elem(&egress_sessions, &key);
		if (sess) {
			if (sess->state == TCP_CONNTRACK_CLOSE || sess->state == TCP_CONNTRACK_TIME_WAIT) {
				/* guest kernel reuse source port too fast */
				del_session(&key, sess);
				goto do_create;
			}

			goto do_update;
		}
do_create:
		/* create new session */
		snat_ip = pick_snat_ip_port(mvm_meta->ip, &key, &snat_port);
		if (!snat_ip || !snat_ip->ip || !snat_port)
			return false;
		ok = create_new_sessions(&key, now, skb->ingress_ifindex, snat_ip, snat_port);
		if (!ok)
			return false;
		sess = bpf_map_lookup_elem(&egress_sessions, &key);
		if (!sess)
			return false;
		goto do_nat;
	} else {
		/* lookup existing session */
		sess = bpf_map_lookup_elem(&egress_sessions, &key);
		if (!sess)
			return false;
	}

do_update:
	/* update session */
	update_session(IP_CT_DIR_ORIGINAL, sess, now, syn, ack, fin, rst);

do_nat:
	/* update L4 */
	old_buff.addr = l3->saddr;
	old_buff.port = l4->source;
	new_buff.addr = sess->node_ip;
	new_buff.port = sess->node_port;
	old_csum = l4->check;
	new_csum = bpf_csum_diff((void *)&old_buff, sizeof(old_buff), (void *)&new_buff, sizeof(new_buff), ~old_csum);
	l4->check = csum_fold(new_csum);
	l4->source = sess->node_port;

	/* update L3 */
	rewrite_l3_addr(l3, &l3->saddr, sess->node_ip);

	/* update L2 */
	set_mac_pair(l2, nodenic_macaddr_p1, nodenic_macaddr_p2,
		     nodegw_macaddr_p1, nodegw_macaddr_p2);

	*dst_ifindex = sess->node_ifindex;
	return true;
}

/* This filter will be attached to the ingress path of Sandbox TAP devices.
 * It performs a SNAT/VXLAN-ENCAP and redirects the packets to target devices.
 */
SEC("tc")
int from_cube(struct __sk_buff *skb)
{
	__u32 daddr, ifindex, dst_ifindex;
	struct mvm_port mvm_port = {};
	struct mvm_meta *mvm_meta;
	struct ethhdr *l2;
	struct iphdr *l3;
	struct tcphdr *l4;
	__u16 *host_port;
	__u8 proto;
	long err;
	int ret;

	skb->queue_mapping = 0;

	/* We handle ETH_P_IP/ETH_P_ARP protocols ONLY */
	if (skb->protocol != bpf_htons(ETH_P_IP)) {
		/* Handle ARP requests with ARP proxy */
		if (skb->protocol == bpf_htons(ETH_P_ARP))
			return handle_arp(skb, skb->ingress_ifindex);
		return TC_ACT_SHOT;
	}

	ifindex = skb->ingress_ifindex;
	mvm_meta = bpf_map_lookup_elem(&ifindex_to_mvmmeta, &ifindex);
	if (!mvm_meta)
		return TC_ACT_SHOT;

	ret = pull_headers(skb, &l2, &l3);
	if (ret != TC_ACT_OK)
		return ret;

	daddr = l3->daddr;
	proto = l3->protocol;

	err = snat(skb, l3, mvm_meta->ip);
	if (err)
		return TC_ACT_SHOT;

	if (daddr == mvm_gateway_ip) {
		/* Cube → host on the cube-dev overlay.  Allow ICMP and TCP
		 * (including SYN) so the sandbox can initiate connections to
		 * host-local services bound on cubegw0_ip — the symmetric
		 * counterpart of from_envoy's host→sandbox path on cube-dev.
		 *
		 * Sandboxes must target mvm_gateway_ip (169.254.68.5), not
		 * cubegw0_ip directly: from_envoy rewrites the reply's source
		 * to mvm_gateway_ip, so anything else creates an asymmetric
		 * 5-tuple that the sandbox's TCP stack RSTs on arrival.
		 */
		switch (proto) {
		case IPPROTO_ICMP:
			break;
		case IPPROTO_TCP:
			break;
		default:
			return TC_ACT_SHOT;
		}

		ret = pull_headers(skb, &l2, &l3);
		if (ret != TC_ACT_OK)
			return ret;

		err = dnat(skb, l3, cubegw0_ip);
		if (err)
			return TC_ACT_SHOT;

		return bpf_redirect(cubegw0_ifindex, BPF_F_INGRESS);
	}

	if (proto == IPPROTO_TCP) {
		if (!__pull_headers(skb, &l2, &l3, &l4))
			return TC_ACT_SHOT;

		mvm_port.ifindex = ifindex;
		mvm_port.listen_port = l4->source;
		host_port = bpf_map_lookup_elem(&local_port_mapping, &mvm_port);
		if (host_port) {
			err = snat_tcp(skb, ifindex, l2, l3, l4, l4->source, *host_port);
			if (err)
				return TC_ACT_SHOT;

			return bpf_redirect(nodenic_ifindex, 0);
		}
	}

	/* Enforce egress network policy before NAT */
	if (!check_net_policy(ifindex, daddr))
		return TC_ACT_SHOT;

	ret = pull_headers(skb, &l2, &l3);
	if (ret != TC_ACT_OK)
		return ret;

	if (should_do_nat(l3)) {
		if (proto == IPPROTO_TCP) {
			if (do_tcp_nat(skb, mvm_meta, &dst_ifindex))
				return bpf_redirect(dst_ifindex, 0);
		}
		if (proto == IPPROTO_UDP) {
			if (do_udp_nat(skb, mvm_meta, &dst_ifindex))
				return bpf_redirect(dst_ifindex, 0);
		}
		if (proto == IPPROTO_ICMP) {
			if (do_icmp_nat(skb, mvm_meta, &dst_ifindex))
				return bpf_redirect(dst_ifindex, 0);
		}
	}

	return TC_ACT_SHOT;
}

char __license[] SEC("license") = "Dual BSD/GPL";
