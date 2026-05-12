// Copyright (c) 2024 Tencent Inc.
// SPDX-License-Identifier: Apache-2.0
//

package service

import (
	"fmt"
	"net"
	"testing"

	"github.com/vishvananda/netlink"
	"golang.org/x/sys/unix"
)

func TestGetGatewayMacAddrAcceptsUsableNeighborStates(t *testing.T) {
	originalLinkByName := netlinkLinkByName
	originalNeighList := netlinkNeighList
	defer func() {
		netlinkLinkByName = originalLinkByName
		netlinkNeighList = originalNeighList
	}()

	netlinkLinkByName = func(name string) (netlink.Link, error) {
		if name != "eth0" {
			t.Fatalf("link name=%q, want eth0", name)
		}
		return &netlink.Dummy{LinkAttrs: netlink.LinkAttrs{Index: 7, Name: name}}, nil
	}
	wantMAC := "02:00:00:00:00:01"
	hw, err := net.ParseMAC(wantMAC)
	if err != nil {
		t.Fatal(err)
	}
	netlinkNeighList = func(index int, family int) ([]netlink.Neigh, error) {
		if index != 7 {
			t.Fatalf("neigh index=%d, want 7", index)
		}
		return []netlink.Neigh{{
			Family:       netlink.FAMILY_V4,
			State:        unix.NUD_STALE,
			HardwareAddr: hw,
		}}, nil
	}

	got, err := getGatewayMacAddr("eth0")
	if err != nil {
		t.Fatalf("getGatewayMacAddr error=%v", err)
	}
	if got != wantMAC {
		t.Fatalf("gateway mac=%q, want %q", got, wantMAC)
	}
}

func TestGetGatewayMacAddrRejectsFailedNeighbor(t *testing.T) {
	originalLinkByName := netlinkLinkByName
	originalNeighList := netlinkNeighList
	defer func() {
		netlinkLinkByName = originalLinkByName
		netlinkNeighList = originalNeighList
	}()

	netlinkLinkByName = func(name string) (netlink.Link, error) {
		return &netlink.Dummy{LinkAttrs: netlink.LinkAttrs{Index: 7, Name: name}}, nil
	}
	hw, err := net.ParseMAC("02:00:00:00:00:01")
	if err != nil {
		t.Fatal(err)
	}
	netlinkNeighList = func(index int, family int) ([]netlink.Neigh, error) {
		return []netlink.Neigh{{
			Family:       netlink.FAMILY_V4,
			State:        unix.NUD_FAILED,
			HardwareAddr: hw,
		}}, nil
	}

	if _, err := getGatewayMacAddr("eth0"); err == nil {
		t.Fatal("getGatewayMacAddr error=nil, want failure")
	} else if got := fmt.Sprint(err); got != "usable gateway mac not found on eth0" {
		t.Fatalf("error=%q", got)
	}
}

func TestEnsureRouteToCubeDev(t *testing.T) {
	originalReplace := netlinkRouteReplace
	originalList := netlinkRouteListFiltered
	defer func() {
		netlinkRouteReplace = originalReplace
		netlinkRouteListFiltered = originalList
	}()

	var got *netlink.Route
	netlinkRouteListFiltered = func(_ int, _ *netlink.Route, _ uint64) ([]netlink.Route, error) {
		return nil, nil
	}
	netlinkRouteReplace = func(route *netlink.Route) error {
		got = route
		return nil
	}

	err := ensureRouteToCubeDev("192.168.0.0/18", &cubeDev{
		Index: 7,
		Name:  cubeDevName,
		IP:    net.ParseIP("192.168.0.1").To4(),
	})
	if err != nil {
		t.Fatalf("ensureRouteToCubeDev error=%v", err)
	}
	if got == nil {
		t.Fatal("route=nil, want route to be installed")
	}
	if got.LinkIndex != 7 {
		t.Fatalf("LinkIndex=%d, want 7", got.LinkIndex)
	}
	if got.Dst == nil || got.Dst.String() != "192.168.0.0/18" {
		t.Fatalf("Dst=%v, want 192.168.0.0/18", got.Dst)
	}
	if got.Scope != netlink.SCOPE_LINK {
		t.Fatalf("Scope=%d, want %d", got.Scope, netlink.SCOPE_LINK)
	}
	if got.Protocol != unix.RTPROT_STATIC {
		t.Fatalf("Protocol=%d, want %d", got.Protocol, unix.RTPROT_STATIC)
	}
}

func TestEnsureRouteToCubeDevSkipsExistingRoute(t *testing.T) {
	originalReplace := netlinkRouteReplace
	originalList := netlinkRouteListFiltered
	defer func() {
		netlinkRouteReplace = originalReplace
		netlinkRouteListFiltered = originalList
	}()

	netlinkRouteListFiltered = func(_ int, route *netlink.Route, _ uint64) ([]netlink.Route, error) {
		return []netlink.Route{{
			LinkIndex: route.LinkIndex,
			Dst:       route.Dst,
			Scope:     route.Scope,
		}}, nil
	}
	netlinkRouteReplace = func(_ *netlink.Route) error {
		t.Fatal("route replace should not be called when route already exists")
		return nil
	}

	err := ensureRouteToCubeDev("192.168.0.0/18", &cubeDev{Index: 7, Name: cubeDevName})
	if err != nil {
		t.Fatalf("ensureRouteToCubeDev error=%v", err)
	}
}

func TestEnsureRouteToCubeDevRejectsInvalidCIDR(t *testing.T) {
	err := ensureRouteToCubeDev("bad-cidr", &cubeDev{Index: 7, Name: cubeDevName})
	if err == nil {
		t.Fatal("ensureRouteToCubeDev error=nil, want invalid cidr")
	}
}

func TestEnsureRouteToCubeDevRequiresDevice(t *testing.T) {
	err := ensureRouteToCubeDev("192.168.0.0/18", nil)
	if err == nil {
		t.Fatal("ensureRouteToCubeDev error=nil, want missing device")
	}
}
