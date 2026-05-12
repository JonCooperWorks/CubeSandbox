// Copyright (c) 2024 Tencent Inc.
// SPDX-License-Identifier: Apache-2.0
//

package cube

import (
	"context"
	"errors"
	"fmt"
	"maps"
	"strconv"
	"time"

	"github.com/tencentcloud/CubeSandbox/CubeMaster/api/services/cubebox/v1"
	"github.com/tencentcloud/CubeSandbox/CubeMaster/pkg/base/config"
	"github.com/tencentcloud/CubeSandbox/CubeMaster/pkg/base/constants"
	"github.com/tencentcloud/CubeSandbox/CubeMaster/pkg/base/log"
	"github.com/tencentcloud/CubeSandbox/CubeMaster/pkg/base/utils"
	"github.com/tencentcloud/CubeSandbox/CubeMaster/pkg/service/sandbox/types"
	"github.com/tencentcloud/CubeSandbox/CubeMaster/pkg/templatecenter"
)

func getCubeboxReqTemplate() (*types.CreateCubeSandboxReq, error) {
	cfg := config.GetConfig()
	if cfg == nil || cfg.ReqTemplateConf == nil || cfg.ReqTemplateConf.CubeBoxReqTemplate == "" {
		return nil, errors.New("cubebox instance type requires CubeBoxReqTemplate configuration")
	}

	templateReq := &types.CreateCubeSandboxReq{}
	err := utils.JSONTool.UnmarshalFromString(config.GetConfig().ReqTemplateConf.CubeBoxReqTemplate, templateReq)
	if err != nil {
		return nil, fmt.Errorf("failed to unmarshal CubeBoxReqTemplate: %w", err)
	}

	return templateReq, nil
}

//go:noinline
func dealCubeboxReqTemplateByLocalConfig(ctx context.Context, reqInOut *types.CreateCubeSandboxReq) error {
	if reqInOut.InstanceType != cubebox.InstanceType_cubebox.String() {
		return nil
	}

	if config.GetConfig().ReqTemplateConf == nil || config.GetConfig().ReqTemplateConf.CubeBoxReqTemplate == "" {
		return errors.New("cubebox instance type requires CubeBoxReqTemplate configuration")
	}

	templateReq, err := getCubeboxReqTemplate()
	if err != nil {
		return fmt.Errorf("failed to unmarshal CubeBoxReqTemplate: %w", err)
	}

	if err := validateContainerRequirements(reqInOut); err != nil {
		return err
	}
	if err := validateTemplateRequirements(templateReq, reqInOut); err != nil {
		return err
	}

	dealVolumeTemplate(reqInOut.Volumes, templateReq.Volumes)

	for i, ctr := range reqInOut.Containers {
		if err := applyTemplateToContainer(ctr, templateReq.Containers[i], i); err != nil {
			return err
		}
	}

	applyTemplateAnnotationsAndLabels(templateReq, reqInOut)
	reqInOut.CubeVSContext = mergeCubeVSContexts(templateReq.CubeVSContext, reqInOut.CubeVSContext)

	if templateReq.NetworkType != "" {
		reqInOut.NetworkType = templateReq.NetworkType
	}

	log.G(ctx).Infof("Successfully dealCubeboxReqTemplateByLocalConfig: %s", utils.InterfaceToString(reqInOut))
	return nil
}

func validateContainerRequirements(req *types.CreateCubeSandboxReq) error {
	if len(req.Volumes) <= 0 {
		return errors.New("volume configuration is required")
	}
	if len(req.Containers) <= 0 {
		return errors.New("at least one container is required")
	}
	return nil
}

func validateTemplateRequirements(templateReq *types.CreateCubeSandboxReq, req *types.CreateCubeSandboxReq) error {
	if len(templateReq.Containers) < len(req.Containers) {
		return fmt.Errorf("template containers count (%d) is less than request containers count (%d)",
			len(templateReq.Containers), len(req.Containers))
	}
	return nil
}

func applyTemplateToContainer(ctr *types.Container, templateCtr *types.Container, index int) error {
	if ctr.Name == "" {
		ctr.Name = templateCtr.Name
		if ctr.Name == "" {
			ctr.Name = "cubebox_" + strconv.Itoa(index)
		}
	}

	if ctr.Image == nil {
		ctr.Image = &types.ImageSpec{}
	}
	applyTemplateImageSpec(templateCtr.Image, ctr.Image)

	if ctr.Resources == nil {
		ctr.Resources = &types.Resource{}
	}
	applyTemplateResources(templateCtr.Resources, ctr.Resources)

	ctr.Syscalls = templateCtr.Syscalls
	ctr.Sysctls = templateCtr.Sysctls
	ctr.SecurityContext = templateCtr.SecurityContext

	ctr.Envs = append(ctr.Envs, templateCtr.Envs...)
	applyTemplateVolumeMounts(templateCtr, ctr)

	if !isContainerReqWhiteTag("WorkingDir") {
		ctr.WorkingDir = templateCtr.WorkingDir
	}

	if !isContainerReqWhiteTag("RLimit") {
		ctr.RLimit = templateCtr.RLimit
	}
	if !isContainerReqWhiteTag("DnsConfig") {
		ctr.DnsConfig = templateCtr.DnsConfig
	}
	if !isContainerReqWhiteTag("HostAliases") {
		ctr.HostAliases = templateCtr.HostAliases
	}
	if !isContainerReqWhiteTag("Poststop") {
		ctr.Poststop = templateCtr.Poststop
	}
	if !isContainerReqWhiteTag("Prestop") {
		ctr.Prestop = templateCtr.Prestop
	}

	return nil
}

func applyTemplateVolumeMounts(templateCtr *types.Container, ctr *types.Container) {

	existNames := make(map[string]struct{})
	existPaths := make(map[string]struct{})
	for _, vm := range ctr.VolumeMounts {
		if vm == nil {
			continue
		}
		if vm.Name != "" {
			existNames[vm.Name] = struct{}{}
		}
		if vm.ContainerPath != "" {
			existPaths[vm.ContainerPath] = struct{}{}
		}
	}

	for _, vm := range templateCtr.VolumeMounts {
		if vm == nil {
			continue
		}
		_, nameExist := existNames[vm.Name]
		_, pathExist := existPaths[vm.ContainerPath]
		if !nameExist && !pathExist {
			ctr.VolumeMounts = append(ctr.VolumeMounts, vm)
			if vm.Name != "" {
				existNames[vm.Name] = struct{}{}
			}
			if vm.ContainerPath != "" {
				existPaths[vm.ContainerPath] = struct{}{}
			}
		}
	}
}

func applyTemplateResources(resourceIn *types.Resource, resourceOut *types.Resource) {
	if resourceIn == nil {
		return
	}
	if resourceOut == nil {
		resourceOut = &types.Resource{}
	}
	if resourceIn.Cpu != "" {
		resourceOut.Cpu = resourceIn.Cpu
	}
	if resourceIn.Mem != "" {
		resourceOut.Mem = resourceIn.Mem
	}
	if resourceIn.Limit != nil {
		resourceOut.Limit = resourceIn.Limit
	}
}

func applyTemplateImageSpec(imageSpecIn *types.ImageSpec, imageSpecOut *types.ImageSpec) {
	if imageSpecIn == nil {
		return
	}
	if imageSpecOut == nil {

		return
	}
	if imageSpecOut.StorageMedia == "" {

		imageSpecOut.StorageMedia = imageSpecIn.StorageMedia
	}

	if imageSpecIn.Image != "" {
		imageSpecOut.Image = imageSpecIn.Image
	}
	if imageSpecIn.Token != "" {
		imageSpecOut.Token = imageSpecIn.Token
	}
	if imageSpecIn.Name != "" {
		imageSpecOut.Name = imageSpecIn.Name
	}
	if imageSpecIn.Annotations != nil {
		if imageSpecOut.Annotations == nil {
			imageSpecOut.Annotations = make(map[string]string)
		}
		maps.Copy(imageSpecOut.Annotations, imageSpecIn.Annotations)
	}
}

//go:noinline
func applyTemplateAnnotationsAndLabels(reqIn *types.CreateCubeSandboxReq, reqOut *types.CreateCubeSandboxReq) {
	if reqIn.Annotations != nil {
		if reqOut.Annotations == nil {
			reqOut.Annotations = make(map[string]string)
		}
		for k, v := range reqIn.Annotations {
			if k == constants.AnnotationsNetID {
				if _, ok := reqOut.Annotations[constants.AnnotationsNetID]; ok {

					continue
				}
			}
			reqOut.Annotations[k] = v
		}
	}

	if reqIn.Labels != nil {
		if reqOut.Labels == nil {
			reqOut.Labels = make(map[string]string)
		}
		maps.Copy(reqOut.Labels, reqIn.Labels)
	}
}

func mergeCubeVSContexts(templateCtx *types.CubeVSContext, requestCtx *types.CubeVSContext) *types.CubeVSContext {
	switch {
	case templateCtx == nil:
		return cloneCubeVSContext(requestCtx)
	case requestCtx == nil:
		return cloneCubeVSContext(templateCtx)
	}

	out := cloneCubeVSContext(templateCtx)
	if requestCtx.AllowInternetAccess != nil {
		allowInternetAccess := *requestCtx.AllowInternetAccess
		out.AllowInternetAccess = &allowInternetAccess
	}
	if len(requestCtx.AllowOut) > 0 {
		out.AllowOut = appendUniqueCIDRs(out.AllowOut, requestCtx.AllowOut)
	}
	if len(requestCtx.DenyOut) > 0 {
		out.DenyOut = appendUniqueCIDRs(out.DenyOut, requestCtx.DenyOut)
	}
	return out
}

func cloneCubeVSContext(in *types.CubeVSContext) *types.CubeVSContext {
	if in == nil {
		return nil
	}
	out := &types.CubeVSContext{
		AllowOut: append([]string(nil), in.AllowOut...),
		DenyOut:  append([]string(nil), in.DenyOut...),
	}
	if in.AllowInternetAccess != nil {
		allowInternetAccess := *in.AllowInternetAccess
		out.AllowInternetAccess = &allowInternetAccess
	}
	return out
}

func appendUniqueCIDRs(base []string, extra []string) []string {
	seen := make(map[string]struct{}, len(base)+len(extra))
	out := append([]string(nil), base...)
	for _, cidr := range base {
		seen[cidr] = struct{}{}
	}
	for _, cidr := range extra {
		if cidr == "" {
			continue
		}
		if _, ok := seen[cidr]; ok {
			continue
		}
		seen[cidr] = struct{}{}
		out = append(out, cidr)
	}
	return out
}

func isContainerReqWhiteTag(tag string) bool {
	if config.GetConfig().ReqTemplateConf == nil || config.GetConfig().ReqTemplateConf.WhitelistReqTag == nil {
		return false
	}
	whitelistReqTag := config.GetConfig().ReqTemplateConf.WhitelistReqTag
	_, ok := whitelistReqTag[tag]
	return ok
}

//go:noinline
func dealCubeboxCreateReqWithTemplate(ctx context.Context, reqInOut *types.CreateCubeSandboxReq) error {

	if reqInOut.InstanceType != cubebox.InstanceType_cubebox.String() {
		return nil
	}
	constants.NormalizeAppSnapshotAnnotations(reqInOut.Annotations)

	// When the caller is restoring from a runtime snapshot it supplies
	// the on-disk path directly via cube.vm.snapshot.base.path and
	// cube.appsnapshot.restore (consumed by the shim). The restore
	// request still arrives with empty Containers/Volumes, so we have
	// to hydrate them from the original template (looked up via
	// cube.master.appsnapshot.template.id) before falling through to
	// cold-start compat for netID. The two shim annotations are
	// re-asserted after template merge in case the template happens
	// to define them.
	if _, ok := reqInOut.Annotations[shimAnnoAppSnapshotRestore]; ok {
		preserved := preserveShimRestoreAnnotations(reqInOut.Annotations)
		templateID := reqInOut.Annotations[constants.CubeAnnotationAppSnapshotTemplateID]
		if templateID != "" {
			if err := dealCubeboxCreateReqWithTemplateCenter(ctx, templateID, reqInOut); err != nil {
				return fmt.Errorf("snapshot-restore template hydrate: %w", err)
			}
		}
		restoreShimRestoreAnnotations(reqInOut, preserved)
		return handleColdStartCompatibility(reqInOut)
	}

	templateID, hasTemplateID := reqInOut.Annotations[constants.CubeAnnotationAppSnapshotTemplateID]

	if !hasTemplateID && config.GetConfig().Common.EnableAGSColdStartSwitch {
		return handleColdStartCompatibility(reqInOut)
	}

	if constants.GetAppSnapshotVersion(reqInOut.Annotations) == templatecenter.DefaultTemplateVersion {
		return dealCubeboxCreateReqWithTemplateCenter(ctx, templateID, reqInOut)
	}

	return dealCubeboxReqTemplateByLocalConfig(ctx, reqInOut)
}

// preserveShimRestoreAnnotations snapshots the two shim-restore
// annotations so the template-merge step (which can copy template
// annotations on top of request annotations) cannot accidentally
// shadow them. Returns the keys actually present.
func preserveShimRestoreAnnotations(in map[string]string) map[string]string {
	if in == nil {
		return nil
	}
	out := map[string]string{}
	for _, k := range []string{shimAnnoAppSnapshotRestore, shimAnnoVMSnapshotBasePath} {
		if v, ok := in[k]; ok {
			out[k] = v
		}
	}
	return out
}

func restoreShimRestoreAnnotations(req *types.CreateCubeSandboxReq, preserved map[string]string) {
	if len(preserved) == 0 {
		return
	}
	if req.Annotations == nil {
		req.Annotations = make(map[string]string, len(preserved))
	}
	for k, v := range preserved {
		req.Annotations[k] = v
	}
}

// shim annotation keys consumed by CubeShim (cube.vm.snapshot.base.path
// + cube.appsnapshot.restore). Defined in
// CubeShim/shim/src/sandbox/config.rs:27,29.
const (
	shimAnnoVMSnapshotBasePath = "cube.vm.snapshot.base.path"
	shimAnnoAppSnapshotRestore = "cube.appsnapshot.restore"
)

// defaultColdStartNetID is the value handleColdStartCompatibility
// injects when neither the inbound request nor the host's
// CubeBoxReqTemplate carries a `com.netid` annotation. Stock cube
// installs ship a CubeBoxReqTemplate with no annotations block at all,
// which used to make every restore-from-snapshot 5xx with
// `netID is missing in CubeBoxReqTemplate`. Downstream consumers only
// require a non-empty string, so a stable default unblocks the
// restore path without forcing operators to patch host config.
const defaultColdStartNetID = "default"

func handleColdStartCompatibility(reqInOut *types.CreateCubeSandboxReq) error {

	if _, hasNetID := reqInOut.Annotations[constants.AnnotationsNetID]; hasNetID {
		return nil
	}

	if reqInOut.Annotations == nil {
		reqInOut.Annotations = make(map[string]string)
	}

	templateReq, err := getCubeboxReqTemplate()
	if err != nil {
		// Even a missing CubeBoxReqTemplate must not wedge the cold
		// start: synthesise the default so the restore can proceed.
		reqInOut.Annotations[constants.AnnotationsNetID] = defaultColdStartNetID
		return nil
	}
	if netID, ok := templateReq.Annotations[constants.AnnotationsNetID]; ok && netID != "" {
		reqInOut.Annotations[constants.AnnotationsNetID] = netID
		return nil
	}
	reqInOut.Annotations[constants.AnnotationsNetID] = defaultColdStartNetID
	return nil
}

//go:noinline
func dealCubeboxCreateReqWithTemplateCenter(ctx context.Context, templateID string, reqInOut *types.CreateCubeSandboxReq) error {
	start := time.Now()
	defer func() {
		templatecenter.ReportResolveMetric(ctx, time.Since(start))
	}()
	if templateID == "" {
		return errors.New("templateID is empty")
	}
	templateReq, err := getTemplateRequestFn(ctx, templateID)
	if err != nil {
		return fmt.Errorf("failed to get template param from store: %w", err)
	}
	constants.NormalizeAppSnapshotAnnotations(templateReq.Annotations)
	if err = templatecenter.EnsureTemplateLocalityReady(ctx, templateID, reqInOut.InstanceType); err != nil {
		return fmt.Errorf("template %s is not ready on any healthy node: %w", templateID, err)
	}
	if log.IsDebug() {
		log.G(ctx).Debugf("getTemplateParam success:%s", utils.InterfaceToString(templateReq))
	} else {
		log.G(ctx).Infof("getTemplateParam success:template=%s %s", templateID, summarizeTemplateRequest(templateReq))
	}

	applyTemplateAnnotationsAndLabels(templateReq, reqInOut)
	reqInOut.CubeVSContext = mergeCubeVSContexts(templateReq.CubeVSContext, reqInOut.CubeVSContext)

	reqInOut.Volumes = append(reqInOut.Volumes, templateReq.Volumes...)

	for i, templateCtr := range templateReq.Containers {
		if len(reqInOut.Containers) <= i {

			reqInOut.Containers = append(reqInOut.Containers, templateCtr)
			continue
		}
		if err := applyTemplateToContainer(reqInOut.Containers[i], templateCtr, i); err != nil {
			return err
		}
	}

	if templateReq.NetworkType != "" {
		reqInOut.NetworkType = templateReq.NetworkType
	}
	if templateReq.RuntimeHandler != "" {
		reqInOut.RuntimeHandler = templateReq.RuntimeHandler
	}
	if templateReq.Namespace != "" {
		reqInOut.Namespace = templateReq.Namespace
	}
	if reqInOut.Labels == nil {
		reqInOut.Labels = map[string]string{}
	}
	if reqInOut.Annotations != nil && reqInOut.Annotations[constants.CubeAnnotationAppSnapshotTemplateID] != "" {
		reqInOut.Labels[constants.CubeAnnotationAppSnapshotTemplateID] = reqInOut.Annotations[constants.CubeAnnotationAppSnapshotTemplateID]
	}
	if log.IsDebug() {
		log.G(ctx).Debugf("dealCubeboxCreateReqWithTemplateCenter success:%s", utils.InterfaceToString(reqInOut))
	} else {
		log.G(ctx).Infof("dealCubeboxCreateReqWithTemplateCenter success:template=%s %s", templateID, summarizeTemplateRequest(reqInOut))
	}
	return nil
}

func summarizeTemplateRequest(req *types.CreateCubeSandboxReq) string {
	if req == nil {
		return "request=nil"
	}
	return fmt.Sprintf(
		"containers=%d volumes=%d labels=%d annotations=%d network=%s runtime=%s namespace=%s cubevs_context=%s",
		len(req.Containers),
		len(req.Volumes),
		len(req.Labels),
		len(req.Annotations),
		req.NetworkType,
		req.RuntimeHandler,
		req.Namespace,
		formatCubeVSContextSummary(req.CubeVSContext),
	)
}

func formatCubeVSContextSummary(ctx *types.CubeVSContext) string {
	if ctx == nil {
		return "allow_internet_access=default(true) allow_out=[] deny_out=[]"
	}
	allowInternetAccess := "default(true)"
	if ctx.AllowInternetAccess != nil {
		allowInternetAccess = fmt.Sprintf("%t", *ctx.AllowInternetAccess)
	}
	return fmt.Sprintf("allow_internet_access=%s allow_out=%v deny_out=%v", allowInternetAccess, ctx.AllowOut, ctx.DenyOut)
}

func dealVolumeTemplate(volumes []*types.Volume, templateVolumes []*types.Volume) {
	for _, v := range volumes {

		if v.VolumeSource != nil && v.VolumeSource.EmptyDir != nil {

			if v.Name == "" && v.VolumeSource.EmptyDir.Medium == 0 {
				templateV := getTemplateVolumes(v.VolumeSource.EmptyDir, templateVolumes)
				if templateV != nil && templateV.VolumeSource != nil && templateV.VolumeSource.EmptyDir != nil {
					v.Name = templateV.Name
					if v.VolumeSource.EmptyDir != nil {
						v.VolumeSource.EmptyDir.Medium = templateV.VolumeSource.EmptyDir.Medium
					}
				}
			}
		}
	}
}

func getTemplateVolumes(sourceVolume interface{}, templateVolumes []*types.Volume) *types.Volume {

	for _, templateVolume := range templateVolumes {
		if templateVolume == nil || templateVolume.VolumeSource == nil {
			continue
		}

		templateSource := templateVolume.VolumeSource

		switch v := sourceVolume.(type) {
		case *types.EmptyDirVolumeSource:
			if v != nil && templateSource.EmptyDir != nil {
				return templateVolume
			}
		case *types.HostDirVolumeSources:
			if v != nil && templateSource.HostDirVolumeSources != nil {
				return templateVolume
			}
		case *types.SandboxPathVolumeSource:
			if v != nil && templateSource.SandboxPath != nil {
				return templateVolume
			}
		}
	}

	return nil
}
