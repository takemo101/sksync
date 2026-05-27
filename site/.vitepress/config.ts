import { defineConfig } from "vitepress";

export default defineConfig({
	title: "sksync",
	description: "Sync Agent Skills symlinks across coding agents from one config file",
	lang: "en-US",
	base: "/sksync/",
	lastUpdated: true,
	cleanUrls: true,
	srcDir: ".",
	outDir: ".vitepress/dist",
	cacheDir: ".vitepress/cache",
	head: [
		["meta", { name: "theme-color", content: "#3b82f6" }],
	],
	themeConfig: {
		nav: [
			{ text: "Quickstart", link: "/quickstart" },
			{ text: "Install", link: "/install" },
			{
				text: "Guides",
				items: [
					{ text: "Project Config (sksync.config.json)", link: "/guides/project-config" },
					{ text: "Agent Mappings (agents.json)", link: "/guides/agent-mappings" },
					{ text: "Sources & Discovery", link: "/guides/sources" },
					{ text: "Bundles", link: "/guides/bundles" },
					{ text: "Lockfile & Sync", link: "/guides/lockfile" },
				],
			},
			{ text: "Commands", link: "/reference/commands" },
			{
				text: "v0.0.7",
				items: [
					{
						text: "Changelog",
						link: "https://github.com/takemo101/sksync/releases",
					},
					{
						text: "GitHub",
						link: "https://github.com/takemo101/sksync",
					},
				],
			},
		],
		sidebar: {
			"/": [
				{
					text: "Getting Started",
					items: [
						{ text: "Quickstart", link: "/quickstart" },
						{ text: "Install", link: "/install" },
					],
				},
				{
					text: "Guides",
					items: [
						{ text: "Project Config (sksync.config.json)", link: "/guides/project-config" },
						{ text: "Agent Mappings (agents.json)", link: "/guides/agent-mappings" },
						{ text: "Sources & Discovery", link: "/guides/sources" },
						{ text: "Bundles", link: "/guides/bundles" },
						{ text: "Lockfile & Sync", link: "/guides/lockfile" },
					],
				},
				{
					text: "Reference",
					items: [{ text: "Commands", link: "/reference/commands" }],
				},
			],
		},
		socialLinks: [{ icon: "github", link: "https://github.com/takemo101/sksync" }],
		editLink: {
			pattern: "https://github.com/takemo101/sksync/edit/main/site/:path",
			text: "Edit this page on GitHub",
		},
		footer: {
			message: "Released under the MIT License.",
			copyright: "© 2026 takemo101",
		},
		search: {
			provider: "local",
		},
	},
});
