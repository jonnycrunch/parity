initSidebarItems({"enum":[["SocketError","Error occurred while establising socket or endpoint"]],"fn":[["init_client","Spawns client <`S`> over specified address creates socket and connects endpoint to it for request-reply connections to the service"],["init_duplex_client","Spawns client <`S`> over specified address creates socket and connects endpoint to it for duplex (paired) connections with the service"]],"struct":[["GuardedSocket","struct for guarding `_endpoint` (so that it wont drop) derefs to client `S`"],["NanoSocket","A type-safe socket wrapper around nanomsg's own socket implementation. This provides a safe interface for dealing with initializing the sockets, sending and receiving messages."],["Worker","Generic worker to handle service (binded) sockets"]],"trait":[["IpcConfig","Allows to configure custom version and custom handshake response for ipc host"],["IpcInterface","Allows implementor to be attached to generic worker and dispatch rpc requests over IPC"],["WithSocket","Basically something that needs only socket to be spawned"]]});