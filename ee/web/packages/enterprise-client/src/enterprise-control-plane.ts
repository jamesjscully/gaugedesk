import {
    bearer,
    browserRouteJson,
    controlPlaneBase,
    type RouteJson,
} from "@gaugewright/control-plane-client";
import * as enterprise from "./control-plane-enterprise";
import type {
    ArchetypeApprovalPolicy,
    Billing,
    EnterpriseAdminApi,
    OrgSettings,
    PlacementPolicy,
    SecurityPolicy,
    SsoConnection,
} from "./control-plane-enterprise";

export { controlPlaneBase };

export class EnterpriseControlPlane implements EnterpriseAdminApi {
    private readonly json: RouteJson;

    constructor(base = controlPlaneBase()) {
        this.json = browserRouteJson(base, { bearer });
    }

    adminGetOrg() {
        return enterprise.adminGetOrg(this.json);
    }

    adminSetOrg(settings: OrgSettings) {
        return enterprise.adminSetOrg(this.json, settings);
    }

    adminDomainVerifyToken(domain: string) {
        return enterprise.adminDomainVerifyToken(this.json, domain);
    }

    adminDomainVerify(domain: string) {
        return enterprise.adminDomainVerify(this.json, domain);
    }

    adminGetMembers() {
        return enterprise.adminGetMembers(this.json);
    }

    adminGetSessions() {
        return enterprise.adminGetSessions(this.json);
    }

    adminInvite(member: { authority: string; email?: string; role: string }) {
        return enterprise.adminInvite(this.json, member);
    }

    adminSetRole(id: string, role: string) {
        return enterprise.adminSetRole(this.json, id, role);
    }

    adminDeactivate(id: string) {
        return enterprise.adminDeactivate(this.json, id);
    }

    adminIntegration() {
        return enterprise.adminIntegration(this.json);
    }

    adminGetSso() {
        return enterprise.adminGetSso(this.json);
    }

    adminSetSso(connection: SsoConnection) {
        return enterprise.adminSetSso(this.json, connection);
    }

    adminTestSso(connection: SsoConnection) {
        return enterprise.adminTestSso(this.json, connection);
    }

    adminGetSecurity() {
        return enterprise.adminGetSecurity(this.json);
    }

    adminSetSecurity(policy: SecurityPolicy) {
        return enterprise.adminSetSecurity(this.json, policy);
    }

    adminGetArchetypeApproval() {
        return enterprise.adminGetArchetypeApproval(this.json);
    }

    adminSetArchetypeApproval(policy: ArchetypeApprovalPolicy) {
        return enterprise.adminSetArchetypeApproval(this.json, policy);
    }

    adminGetPlacementPolicy() {
        return enterprise.adminGetPlacementPolicy(this.json);
    }

    adminSetPlacementPolicy(policy: PlacementPolicy) {
        return enterprise.adminSetPlacementPolicy(this.json, policy);
    }

    adminGetBilling() {
        return enterprise.adminGetBilling(this.json);
    }

    adminSetBilling(billing: Billing) {
        return enterprise.adminSetBilling(this.json, billing);
    }

    adminGetAudit() {
        return enterprise.adminGetAudit(this.json);
    }

    adminIssueScimToken() {
        return enterprise.adminIssueScimToken(this.json);
    }
}
