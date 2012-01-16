"""
py2app/py2exe build script for Lightningbeam.

Will automatically ensure that all build prerequisites are available
via ez_setup

Usage (Mac OS X):
    python setup.py py2app

Usage (Windows):
    python setup.py py2exe
"""
import ez_setup
ez_setup.use_setuptools()

import sys
from setuptools import setup


mainscript = 'lightningbeam.py'

if sys.platform == 'darwin':
    extra_options = dict(
        setup_requires=['py2app'],
        app=[mainscript],
        # Cross-platform applications generally expect sys.argv to
        # be used for opening files.
        options=dict(py2app=dict(argv_emulation=False,
        plist=dict(
            #CFBundleDocumentTypes= ,
            CFBundleIdentifyer='org.lightningbeam.lightningbeam',
            #LSPrefersPPC=True,
        ),
        resources=["media","gpl.txt","swfc","base.js"],
        iconfile="Lightningbeam.icns"
        )),
    )
elif sys.platform == 'win32':
    extra_options = dict(
        setup_requires=['py2exe'],
        app=[mainscript],
        options=dict(py2app=dict(resources=["media","gpl.txt","swfc","base.js"],)),
    )
else:
     extra_options = dict(
         # Normally unix-like platforms will use "setup.py install"
         # and install the main script as such
         scripts=[mainscript],
     )

setup(
    name="Lightningbeam",
    **extra_options
)