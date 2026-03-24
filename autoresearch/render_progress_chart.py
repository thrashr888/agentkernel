import csv
from datetime import datetime
import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt
from matplotlib.dates import DateFormatter

rows = []
with open('autoresearch/results_clean.tsv', newline='') as f:
    reader = csv.DictReader(f, delimiter='\t')
    for row in reader:
        row['timestamp_dt'] = datetime.fromisoformat(row['timestamp'].replace('Z', '+00:00'))
        for k in ('total_score','startup_avg_ms','total_avg_ms','throughput_per_second'):
            row[k] = float(row[k])
        rows.append(row)
rows.sort(key=lambda r: r['timestamp_dt'])
backends = sorted(set(r['backend'] for r in rows))
colors = {'docker':'#1f77b4','apple':'#ff7f0e','podman':'#2ca02c','firecracker':'#d62728','hyperlight':'#9467bd'}
fig, axes = plt.subplots(3, 1, figsize=(12, 10), sharex=True)
metrics = [
    ('total_score', 'Total score (higher better)'),
    ('total_avg_ms', 'End-to-end latency ms'),
    ('throughput_per_second', 'Throughput / s'),
]
for ax, (key, label) in zip(axes, metrics):
    for backend in backends:
        pts = [r for r in rows if r['backend'] == backend]
        xs = [r['timestamp_dt'] for r in pts]
        ys = [r[key] for r in pts]
        ax.plot(xs, ys, marker='o', linewidth=2, label=backend, color=colors.get(backend))
        for r in pts:
            ax.annotate(r['git_sha'][:7], (r['timestamp_dt'], r[key]), textcoords='offset points', xytext=(4,5), fontsize=8, alpha=0.75)
    ax.set_ylabel(label)
    ax.grid(True, alpha=0.25)
axes[0].set_title('Agentkernel autoresearch progress')
axes[-1].xaxis.set_major_formatter(DateFormatter('%m-%d\n%H:%M'))
handles, labels = axes[0].get_legend_handles_labels()
if handles:
    fig.legend(handles, labels, loc='upper center', ncol=max(1, len(labels)))
fig.tight_layout(rect=(0, 0, 1, 0.96))
fig.savefig('autoresearch/progress.png', dpi=180)
print('wrote autoresearch/progress.png')
