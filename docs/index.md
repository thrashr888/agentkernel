---
hide:
  - navigation
  - toc
  - footer
---

<main class="ak-home">
  <section class="ak-hero" aria-labelledby="ak-hero-title">
    <div class="ak-shell ak-hero-grid">
      <div class="ak-hero-copy">
        <p class="ak-kicker"><span aria-hidden="true"></span> Open-source runtime for coding agents</p>
        <h1 id="ak-hero-title">Give every coding agent a safe place to work.</h1>
        <p class="ak-lede">AgentKernel runs Claude Code, Codex, Gemini CLI, and the rest inside isolated environments&mdash;with dedicated kernels where available, host-held secrets, and one interface from laptop to cluster.</p>
        <div class="ak-actions">
          <a class="ak-button ak-button-primary" href="getting-started/quick-start/">Start with one command</a>
          <a class="ak-button ak-button-secondary" href="platform-overview/">Read the docs <span aria-hidden="true">&rarr;</span></a>
        </div>
        <ul class="ak-proof" aria-label="Project attributes">
          <li>Open source</li>
          <li>Local first</li>
          <li>Any agent</li>
          <li>CLI + APIs</li>
        </ul>
      </div>

      <aside class="ak-terminal" aria-label="AgentKernel command example">
        <div class="ak-terminal-bar">
          <span class="ak-terminal-title">agentkernel / local</span>
          <span class="ak-terminal-status"><i aria-hidden="true"></i> ready</span>
        </div>
        <div class="ak-terminal-body">
          <p><span class="ak-prompt" aria-hidden="true">$</span> agentkernel sandbox create fix-482 \</p>
          <p class="ak-terminal-indent">--template codex-sandbox</p>
          <p class="ak-terminal-gap"><span class="ak-dim">01</span> policy&nbsp;&nbsp;&nbsp;&nbsp;moderate</p>
          <p><span class="ak-dim">02</span> runtime&nbsp;&nbsp;&nbsp;firecracker</p>
          <p><span class="ak-dim">03</span> secrets&nbsp;&nbsp;&nbsp;host-side</p>
          <p><span class="ak-dim">04</span> workspace&nbsp;/workspace</p>
          <div class="ak-terminal-rule" aria-hidden="true"></div>
          <p><span class="ak-ok" aria-hidden="true">&#10003;</span> sandbox <strong>fix-482</strong> is ready</p>
          <p class="ak-terminal-gap"><span class="ak-prompt" aria-hidden="true">$</span> agentkernel attach fix-482<span class="ak-cursor" aria-hidden="true"></span></p>
        </div>
      </aside>
    </div>
  </section>

  <section class="ak-intro" aria-labelledby="ak-intro-title">
    <div class="ak-shell ak-intro-grid">
      <p class="ak-section-label">Why AgentKernel</p>
      <div>
        <h2 id="ak-intro-title">Agents need a computer.<br>They shouldn&rsquo;t need yours.</h2>
        <p>Useful agents install packages, edit files, launch browsers, and run arbitrary commands. AgentKernel puts that work behind an explicit runtime boundary without asking you to replace the tools you already use.</p>
      </div>
    </div>
  </section>

  <section class="ak-boundary-section" aria-labelledby="ak-boundary-title">
    <div class="ak-shell">
      <div class="ak-section-heading">
        <p class="ak-section-label">The boundary</p>
        <h2 id="ak-boundary-title">The agent gets what it needs.<br>Nothing arrives by accident.</h2>
      </div>

      <div class="ak-boundary" role="img" aria-label="Host resources pass through an AgentKernel policy boundary into an isolated sandbox">
        <div class="ak-boundary-column">
          <p class="ak-boundary-eyebrow">Your machine</p>
          <h3>Keep control</h3>
          <ul>
            <li><span aria-hidden="true">/</span> selected workspace</li>
            <li><span aria-hidden="true">#</span> host-held credentials</li>
            <li><span aria-hidden="true">&sect;</span> explicit policy</li>
          </ul>
        </div>
        <div class="ak-boundary-gate">
          <span>AgentKernel</span>
          <b aria-hidden="true">&rarr;</b>
          <small>allow only what the task needs</small>
        </div>
        <div class="ak-boundary-column ak-boundary-column-dark">
          <p class="ak-boundary-eyebrow">Agent sandbox</p>
          <h3>Let it work</h3>
          <ul>
            <li><span aria-hidden="true">&gt;_</span> shell + packages</li>
            <li><span aria-hidden="true">&#9673;</span> isolated runtime</li>
            <li><span aria-hidden="true">&#8599;</span> scoped network</li>
          </ul>
        </div>
      </div>
    </div>
  </section>

  <section class="ak-capabilities" aria-labelledby="ak-capabilities-title">
    <div class="ak-shell">
      <div class="ak-section-heading ak-section-heading-row">
        <div>
          <p class="ak-section-label">Built for real agent work</p>
          <h2 id="ak-capabilities-title">A runtime, not another harness.</h2>
        </div>
        <p>Bring the agent loop, model, and tools you prefer. AgentKernel owns the risky part: where the code runs and what it can reach.</p>
      </div>

      <div class="ak-feature-list">
        <article>
          <p class="ak-feature-number">01</p>
          <div>
            <h3>Secrets stay outside</h3>
            <p>Credentials can be injected by a host-side proxy only for approved destinations. The sandbox receives a placeholder, not the key.</p>
          </div>
          <a href="features/secrets/">Secret isolation <span aria-hidden="true">&rarr;</span></a>
        </article>
        <article>
          <p class="ak-feature-number">02</p>
          <div>
            <h3>Every agent, one boundary</h3>
            <p>Claude Code, Codex, Gemini CLI, Copilot, Amp, OpenCode, and Pi use the same sandbox lifecycle and policy surface.</p>
          </div>
          <a href="agents/">Agent guides <span aria-hidden="true">&rarr;</span></a>
        </article>
        <article>
          <p class="ak-feature-number">03</p>
          <div>
            <h3>Local when possible, remote when useful</h3>
            <p>Use Firecracker, Apple Containers, Docker, Kubernetes, Nomad, or a hosted provider without teaching the agent a new interface.</p>
          </div>
          <a href="config/backends/">Backend matrix <span aria-hidden="true">&rarr;</span></a>
        </article>
      </div>
    </div>
  </section>

  <section class="ak-state" aria-labelledby="ak-state-title">
    <div class="ak-shell ak-state-grid">
      <div>
        <p class="ak-section-label">State that means what it says</p>
        <h2 id="ak-state-title">Save files, pause a machine, or branch the work.</h2>
        <p>AgentKernel keeps filesystem snapshots separate from full-machine continuation. Automation can choose the contract it actually needs.</p>
        <a class="ak-text-link" href="operations/firecracker-full-state/">Full-state lifecycle preview <span aria-hidden="true">&rarr;</span></a>
      </div>
      <dl class="ak-state-list">
        <div>
          <dt>Snapshot</dt>
          <dd>Preserve files and installed state. Restore with fresh processes.</dd>
        </div>
        <div>
          <dt>Pause <span>Preview</span></dt>
          <dd>Preserve guest memory, devices, processes, and disk on compatible Firecracker hosts.</dd>
        </div>
        <div>
          <dt>Fork <span>Preview</span></dt>
          <dd>Create an independent child from an immutable full-state checkpoint.</dd>
        </div>
      </dl>
    </div>
  </section>

  <section class="ak-backends" aria-labelledby="ak-backends-title">
    <div class="ak-shell">
      <p class="ak-section-label">One control surface</p>
      <h2 id="ak-backends-title">Laptop to cluster, without a rewrite.</h2>
      <div class="ak-backend-row">
        <div><span>Local</span><p>Firecracker<br>Apple Containers<br>Docker / Podman</p></div>
        <div><span>Cluster</span><p>Kubernetes<br>Nomad<br>Warm pools</p></div>
        <div><span>Hosted</span><p>Daytona / E2B<br>Runloop / Modal<br>Remote adapters</p></div>
        <div><span>Interfaces</span><p>CLI / HTTP / MCP<br>Five SDKs<br>Desktop app</p></div>
      </div>
    </div>
  </section>

  <section class="ak-final" aria-labelledby="ak-final-title">
    <div class="ak-shell ak-final-inner">
      <div>
        <p class="ak-section-label">Start small</p>
        <h2 id="ak-final-title">Put one command behind a safer boundary.</h2>
      </div>
      <div>
        <code>brew tap thrashr888/tap &amp;&amp; brew install agentkernel</code>
        <div class="ak-actions">
          <a class="ak-button ak-button-primary" href="getting-started/installation/">Install AgentKernel</a>
          <a class="ak-button ak-button-secondary" href="platform-overview/">Explore the platform</a>
        </div>
      </div>
    </div>
  </section>
</main>
