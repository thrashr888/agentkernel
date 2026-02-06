# Kubernetes Deployment

Deploy agentkernel to Kubernetes using Kustomize or Helm.

## Kustomize (Simple)

```bash
# Deploy
kubectl apply -k kustomize/

# Check status
kubectl -n agentkernel get pods

# Port forward for local access
kubectl -n agentkernel port-forward svc/agentkernel 18888:18888
```

### With Secrets

```bash
# Create API key secret
kubectl -n agentkernel create secret generic agentkernel-secrets \
  --from-literal=AGENTKERNEL_API_KEY=your-secret-key
```

### Customization

Create `kustomize/overlays/production/kustomization.yaml`:

```yaml
apiVersion: kustomize.config.k8s.io/v1beta1
kind: Kustomization

resources:
  - ../../

patches:
  - patch: |-
      - op: replace
        path: /spec/replicas
        value: 3
    target:
      kind: Deployment
      name: agentkernel

images:
  - name: ghcr.io/thrashr888/agentkernel
    newTag: v0.8.0
```

## Helm (Coming Soon)

```bash
helm repo add agentkernel https://thrashr888.github.io/agentkernel
helm install agentkernel agentkernel/agentkernel
```

## Enterprise CRDs

For advanced deployments, agentkernel provides Custom Resource Definitions:

```yaml
apiVersion: agentkernel.io/v1alpha1
kind: AgentKernelPolicy
metadata:
  name: default-policy
spec:
  securityProfile: moderate
  allowedDomains:
    - api.openai.com
    - api.anthropic.com
```

See [Enterprise docs](../../../docs/enterprise.md) for details.

## Requirements

- Kubernetes 1.25+
- Docker socket access (for container backend)
- PersistentVolume provisioner (for data storage)
- Optional: Privileged containers (for Firecracker backend)

## Ingress

Example with nginx-ingress:

```yaml
apiVersion: networking.k8s.io/v1
kind: Ingress
metadata:
  name: agentkernel
  namespace: agentkernel
  annotations:
    nginx.ingress.kubernetes.io/ssl-redirect: "true"
spec:
  ingressClassName: nginx
  tls:
    - hosts:
        - agentkernel.example.com
      secretName: agentkernel-tls
  rules:
    - host: agentkernel.example.com
      http:
        paths:
          - path: /
            pathType: Prefix
            backend:
              service:
                name: agentkernel
                port:
                  number: 18888
```
