use utoipa::openapi::security::{Http, HttpAuthScheme, SecurityScheme};
use utoipa::{Modify, OpenApi};

struct SecurityAddon;

impl Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi
            .components
            .get_or_insert_with(utoipa::openapi::Components::new);

        components.add_security_scheme(
            "bearerAuth",
            SecurityScheme::Http(Http::new(HttpAuthScheme::Bearer)),
        );
    }
}

#[derive(OpenApi)]
#[openapi(
    paths(
        crate::routes::api::system::health,
        crate::routes::api::system::stats,
        crate::routes::api::proxmox::version,
        crate::routes::api::proxmox::nodes,
        crate::routes::api::dhcp::list_leases,
        crate::routes::api::dhcp::assign_lease,
        crate::routes::api::dhcp::delete_lease,
        crate::routes::api::servers::list_servers,
        crate::routes::api::containers::list_containers,
        crate::routes::api::containers::set_root_password,
    ),
    components(
        schemas(
            crate::routes::api::system::HealthResponse,
            crate::routes::api::system::StatsResponse,
            crate::routes::api::system::MemoryStats,
            crate::routes::api::proxmox::ErrorResponse,
            crate::routes::api::proxmox::VersionResponse,
            crate::routes::api::proxmox::NodesResponse,
            crate::routes::api::proxmox::ApiProxmoxVersion,
            crate::routes::api::proxmox::ApiProxmoxNode,
            crate::routes::api::dhcp::ErrorResponse,
            crate::routes::api::dhcp::LeasesResponse,
            crate::routes::api::dhcp::ApiLease,
            crate::routes::api::dhcp::AssignLeaseRequest,
            crate::routes::api::dhcp::AssignLeaseResponse,
            crate::routes::api::dhcp::DeleteLeaseResponse,
            crate::routes::api::servers::ServersResponse,
            crate::routes::api::servers::ApiServer,
            crate::routes::api::servers::ApiServerDhcpStatus,
            crate::routes::api::containers::ContainersResponse,
            crate::routes::api::containers::ApiContainer,
            crate::routes::api::containers::ApiContainerDhcpStatus,
            crate::routes::api::containers::SetRootPasswordRequest,
            crate::routes::api::containers::SetRootPasswordResponse,
        )
    ),
    tags(
        (name = "system", description = "System endpoints"),
        (name = "proxmox", description = "Proxmox read endpoints"),
        (name = "dhcp", description = "DHCP lease management endpoints"),
        (name = "servers", description = "Virtual machine endpoints"),
        (name = "containers", description = "Container endpoints")
    ),
    modifiers(&SecurityAddon),
    security(
        ("bearerAuth" = [])
    )
)]
pub struct ApiDoc;
