/*
 * noVNC: HTML5 VNC client
 * Copyright (C) 2012 Joel Martin
 * Copyright (C) 2015 Samuel Mannehed for Cendio AB
 * Licensed under MPL 2.0 (see LICENSE.txt)
 *
 * See README.md for usage and integration instructions.
 */

/* jslint white: false, browser: true */
/* global window, $D, Util, WebUtil, RFB, Display */
var UI;

(function () {
    "use strict";

    // Load supporting scripts
    window.onscriptsload = function () {
        UI.load();
    };
    Util.load_scripts(["webutil.js"]);

    UI = {
        load: function (callback) {
            var token = WebUtil.getQueryVar('token', null);
            if (token) {
                WebUtil.createCookie('PVEAuthCookie', token, 1);
            }
            var error = WebUtil.getQueryVar('error', null); 
            if (error && error !== undefined && error !== null) {
                error = error.replace(/(<([^>]+)>)/gi, ""); UI.updateState(null, 'failed', 'loaded',
                        error);
                return;
            }
            var consoleType = WebUtil.getQueryVar('console');
            if (WebUtil.getQueryVar('console') == "qemu" || WebUtil.getQueryVar('virtualization') == "qemu") {
                consoleType = "kvm";
            }else if(WebUtil.getQueryVar('virtualization') ){
                consoleType = WebUtil.getQueryVar('virtualization');
            }
            var controller = "novnc=1";
            if (WebUtil.getQueryVar('xtermjs')) {
                controller = "xtermjs=1";
            }
            window.location = "../?console=" + consoleType + "&" + controller + "&vmid=" + WebUtil.getQueryVar('vmid') + "&vmname=" + WebUtil.getQueryVar('vmid') + "&node=" + WebUtil.getQueryVar('node') + "";
        },
        updateState: function (rfb, state, oldstate, msg) {
            var klass;
            switch (state) {
                case 'failed':
                case 'fatal':
                    klass = "noVNC_status_error";
                    break;
                case 'normal':
                    klass = "noVNC_status_normal";
                    UI.pveAllowMigratedVMTest = true;
                    break;
                case 'disconnected':
                    $D('noVNC_logo').style.display = "block";
                    $D('noVNC_container').style.display = "none";
                    /* falls through */
                case 'loaded':
                    klass = "noVNC_status_normal";
                    break;
                default:
                    klass = "noVNC_status_warn";
                    break;
            }
            if (typeof (msg) !== 'undefined') {
                $D('noVNC-control-bar').setAttribute("class", klass);
                $D('noVNC_status').innerHTML = msg;
            }

        },
    };
})();
