// Copyright (c) 2024 Tencent Inc.
// SPDX-License-Identifier: Apache-2.0
//

package cube

import (
	"context"
	"errors"
	"net/http"
	"strings"

	"github.com/google/uuid"
	cubebox "github.com/tencentcloud/CubeSandbox/CubeMaster/api/services/cubebox/v1"
	"github.com/tencentcloud/CubeSandbox/CubeMaster/pkg/base/log"
	"github.com/tencentcloud/CubeSandbox/CubeMaster/pkg/cubelet"
	"github.com/tencentcloud/CubeSandbox/CubeMaster/pkg/errorcode"
	"github.com/tencentcloud/CubeSandbox/CubeMaster/pkg/localcache"
	"github.com/tencentcloud/CubeSandbox/CubeMaster/pkg/service/httpservice/common"
	"github.com/tencentcloud/CubeSandbox/CubeMaster/pkg/service/sandbox"
	"github.com/tencentcloud/CubeSandbox/CubeMaster/pkg/service/sandbox/types"
	"github.com/tencentcloud/CubeSandbox/cubelog"
)

// sandboxSnapshotRequest mirrors the shape CubeAPI already POSTs at
// /cube/sandbox/snapshot (see CubeAPI/src/cubemaster/mod.rs:876).
//
// Either Name or Names[0] is treated as the caller-supplied snapshot
// identifier. If both are empty a UUID is generated. The chosen
// identifier is used as the TemplateID parameter of Cubelet's
// CommitSandbox RPC, which determines the on-disk directory layout.
type sandboxSnapshotRequest struct {
	RequestID    string   `json:"requestID,omitempty"`
	SandboxID    string   `json:"sandboxID,omitempty"`
	InstanceType string   `json:"instanceType,omitempty"`
	Name         string   `json:"name,omitempty"`
	Names        []string `json:"names,omitempty"`
	Sync         bool     `json:"sync,omitempty"`
	Timeout      *int32   `json:"timeout,omitempty"`
}

// sandboxSnapshotResponse mirrors what CubeAPI deserialises (see
// SandboxSnapshotResponse in CubeAPI/src/cubemaster/mod.rs:893). Path
// is a fork-local extension so the orchestrator can persist its own
// snapshot_id -> path mapping without round-tripping a separate query.
type sandboxSnapshotResponse struct {
	*types.Res
	SandboxID  string   `json:"sandboxID,omitempty"`
	SnapshotID string   `json:"snapshot_id,omitempty"`
	Names      []string `json:"names,omitempty"`
	Path       string   `json:"path,omitempty"`
}

// sandboxSnapshotDeleteRequest carries the host that owns the snapshot
// blob. The orchestrator records (snapshot_id, host_ip) at create time
// and supplies both for delete; Cube does not maintain a snapshot
// index of its own.
type sandboxSnapshotDeleteRequest struct {
	RequestID  string `json:"requestID,omitempty"`
	SnapshotID string `json:"snapshot_id,omitempty"`
	HostIP     string `json:"host_ip,omitempty"`
}

type sandboxSnapshotDeleteResponse struct {
	*types.Res
	SnapshotID string `json:"snapshot_id,omitempty"`
}

func handleSandboxSnapshotAction(w http.ResponseWriter, r *http.Request, rt *CubeLog.RequestTrace) interface{} {
	_ = w
	req := &sandboxSnapshotRequest{}
	if err := common.GetBodyReq(r, req); err != nil {
		return &sandboxSnapshotResponse{
			Res: &types.Res{Ret: &types.Ret{
				RetCode: int(errorcode.ErrorCode_MasterParamsError),
				RetMsg:  err.Error(),
			}},
		}
	}
	if strings.TrimSpace(req.SandboxID) == "" {
		return &sandboxSnapshotResponse{
			Res: &types.Res{Ret: &types.Ret{
				RetCode: int(errorcode.ErrorCode_MasterParamsError),
				RetMsg:  "sandboxID is required",
			}},
		}
	}

	snapshotID := strings.TrimSpace(req.Name)
	if snapshotID == "" && len(req.Names) > 0 {
		snapshotID = strings.TrimSpace(req.Names[0])
	}
	if snapshotID == "" {
		snapshotID = uuid.New().String()
	}

	hostIP, err := resolveSandboxHostIP(r.Context(), req.RequestID, req.SandboxID)
	if err != nil {
		return &sandboxSnapshotResponse{
			Res: &types.Res{Ret: &types.Ret{
				RetCode: int(errorcode.ErrorCode_NotFound),
				RetMsg:  err.Error(),
			}},
			SandboxID:  req.SandboxID,
			SnapshotID: snapshotID,
		}
	}

	ctx := log.WithLogger(r.Context(), log.G(r.Context()).WithFields(map[string]any{
		"RequestId":  req.RequestID,
		"Action":     "SandboxSnapshot",
		"SandboxID":  req.SandboxID,
		"SnapshotID": snapshotID,
		"HostIP":     hostIP,
	}))

	calleeEndpoint := cubelet.GetCubeletAddr(hostIP)
	commitRsp, err := cubelet.CommitSandbox(ctx, calleeEndpoint, &cubebox.CommitSandboxRequest{
		RequestID:  req.RequestID,
		SandboxID:  req.SandboxID,
		TemplateID: snapshotID,
	})
	if err != nil {
		return &sandboxSnapshotResponse{
			Res: &types.Res{Ret: &types.Ret{
				RetCode: int(errorcode.ErrorCode_MasterInternalError),
				RetMsg:  err.Error(),
			}},
			SandboxID:  req.SandboxID,
			SnapshotID: snapshotID,
		}
	}
	if commitRsp.GetRet() == nil || commitRsp.GetRet().GetRetCode() != 0 {
		retMsg := "cubelet commit_sandbox failed"
		retCode := int(errorcode.ErrorCode_MasterInternalError)
		if commitRsp.GetRet() != nil {
			retMsg = commitRsp.GetRet().GetRetMsg()
			retCode = int(commitRsp.GetRet().GetRetCode())
		}
		return &sandboxSnapshotResponse{
			Res: &types.Res{Ret: &types.Ret{RetCode: retCode, RetMsg: retMsg}},
			SandboxID:  req.SandboxID,
			SnapshotID: snapshotID,
		}
	}

	names := req.Names
	if len(names) == 0 && req.Name != "" {
		names = []string{req.Name}
	}

	rt.RequestID = req.RequestID
	rt.RetCode = int64(errorcode.ErrorCode_Success)
	return &sandboxSnapshotResponse{
		Res: &types.Res{
			RequestID: req.RequestID,
			Ret:       &types.Ret{RetCode: int(errorcode.ErrorCode_Success), RetMsg: "success"},
		},
		SandboxID:  req.SandboxID,
		SnapshotID: snapshotID,
		Names:      names,
		Path:       commitRsp.GetSnapshotPath(),
	}
}

func handleSandboxSnapshotDeleteAction(w http.ResponseWriter, r *http.Request, rt *CubeLog.RequestTrace) interface{} {
	_ = w
	req := &sandboxSnapshotDeleteRequest{}
	if err := common.GetBodyReq(r, req); err != nil {
		return &sandboxSnapshotDeleteResponse{
			Res: &types.Res{Ret: &types.Ret{
				RetCode: int(errorcode.ErrorCode_MasterParamsError),
				RetMsg:  err.Error(),
			}},
		}
	}
	snapshotID := strings.TrimSpace(req.SnapshotID)
	hostIP := strings.TrimSpace(req.HostIP)
	if snapshotID == "" || hostIP == "" {
		return &sandboxSnapshotDeleteResponse{
			Res: &types.Res{Ret: &types.Ret{
				RetCode: int(errorcode.ErrorCode_MasterParamsError),
				RetMsg:  "snapshot_id and host_ip are required",
			}},
		}
	}

	ctx := log.WithLogger(r.Context(), log.G(r.Context()).WithFields(map[string]any{
		"RequestId":  req.RequestID,
		"Action":     "SandboxSnapshotDelete",
		"SnapshotID": snapshotID,
		"HostIP":     hostIP,
	}))

	calleeEndpoint := cubelet.GetCubeletAddr(hostIP)
	cleanRsp, err := cubelet.CleanupTemplate(ctx, calleeEndpoint, &cubebox.CleanupTemplateRequest{
		RequestID:  req.RequestID,
		TemplateID: snapshotID,
	})
	if err != nil {
		return &sandboxSnapshotDeleteResponse{
			Res: &types.Res{Ret: &types.Ret{
				RetCode: int(errorcode.ErrorCode_MasterInternalError),
				RetMsg:  err.Error(),
			}},
			SnapshotID: snapshotID,
		}
	}
	if cleanRsp.GetRet() == nil || cleanRsp.GetRet().GetRetCode() != 0 {
		retMsg := "cubelet cleanup_template failed"
		retCode := int(errorcode.ErrorCode_MasterInternalError)
		if cleanRsp.GetRet() != nil {
			retMsg = cleanRsp.GetRet().GetRetMsg()
			retCode = int(cleanRsp.GetRet().GetRetCode())
		}
		return &sandboxSnapshotDeleteResponse{
			Res:        &types.Res{Ret: &types.Ret{RetCode: retCode, RetMsg: retMsg}},
			SnapshotID: snapshotID,
		}
	}

	rt.RequestID = req.RequestID
	rt.RetCode = int64(errorcode.ErrorCode_Success)
	return &sandboxSnapshotDeleteResponse{
		Res: &types.Res{
			RequestID: req.RequestID,
			Ret:       &types.Ret{RetCode: int(errorcode.ErrorCode_Success), RetMsg: "success"},
		},
		SnapshotID: snapshotID,
	}
}

// resolveSandboxHostIP mirrors the lookup pattern in template_commit.go:
// localcache first, then SandboxInfo. Returns the host IP that owns
// the sandbox.
func resolveSandboxHostIP(ctx context.Context, requestID, sandboxID string) (string, error) {
	if cache := localcache.GetSandboxCache(sandboxID); cache != nil && cache.HostIP != "" {
		return cache.HostIP, nil
	}
	infoRsp := sandbox.SandboxInfo(ctx, &types.GetCubeSandboxReq{
		RequestID: requestID,
		SandboxID: sandboxID,
	})
	if infoRsp == nil || infoRsp.Ret == nil || infoRsp.Ret.RetCode != int(errorcode.ErrorCode_Success) || len(infoRsp.Data) == 0 {
		msg := "sandbox not found"
		if infoRsp != nil && infoRsp.Ret != nil && infoRsp.Ret.RetMsg != "" {
			msg = infoRsp.Ret.RetMsg
		}
		return "", errors.New(msg)
	}
	if infoRsp.Data[0].HostIP == "" {
		return "", errors.New("sandbox has no host IP")
	}
	return infoRsp.Data[0].HostIP, nil
}
